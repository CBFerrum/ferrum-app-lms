//! Paid courses (the `lms_payments` package, with Commerce). A course with a price is bought on its
//! page: its Buy action invoices the learner and asks for the invoice through the store's payment
//! method, and the payment arriving enrols them. Nobody enrols themselves in a paid course they have
//! not paid for; instructors and LMS managers still enrol anyone. Withdrawing within the refund
//! period, before completing the course, credits the invoice and asks for the payment to go back
//! (a person approves the refund in the store). A purchase whose payment request lapsed unpaid is
//! cancelled with its invoice after a day, or asked for again when the learner buys again.

use crate::enrollment::{self, ACTIVE, Outcome, WITHDRAWN};
use crate::util::{self, b, d, doc_id, elevated, id, load, s, sys};
use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use ferrum_domain::util::{get, money, rows, set};
use ferrum_kernel::{Ctx, Doc, DocHook, Error, HookCtx, Kernel, Result, Tx};
use rust_decimal::Decimal;
use serde_json::{Value, json};
use uuid::Uuid;

pub(crate) const PACKAGE: &str = "lms_payments";

const AWAITING: &str = "Awaiting Payment";
const PAID: &str = "Paid";
const CANCELLED: &str = "Cancelled";
const REFUNDING: &str = "Refund Requested";

/// Whether the workspace sells courses.
pub(crate) fn installed(ctx: &Ctx) -> bool {
    ctx.schema.packages.iter().any(|p| p == PACKAGE)
}

#[derive(Default)]
struct Terms {
    company: Option<Uuid>,
    method: Option<String>,
    refund_days: i64,
}

async fn terms(tx: &mut Tx, tenant: Uuid) -> Result<Terms> {
    let row: Option<(Option<Uuid>, Option<String>, Option<i64>)> =
        sqlx::query_as("select lpay_company, lpay_payment_method, lpay_refund_days from e_lms_settings where tenant_id = $1 limit 1")
            .bind(tenant)
            .fetch_optional(&mut **tx)
            .await?;
    let (company, method, days) = row.unwrap_or_default();
    Ok(Terms { company, method: method.map(|m| m.trim().to_string()).filter(|m| !m.is_empty()), refund_days: days.unwrap_or(14).max(0) })
}

/// The code of the currency courses are sold in: the selling company's.
async fn currency(tx: &mut Tx, tenant: Uuid) -> Result<Option<String>> {
    Ok(sqlx::query_scalar(
        "select cur.code from e_lms_settings st join e_company co on co.tenant_id = st.tenant_id and co.id = st.lpay_company \
         join e_currency cur on cur.tenant_id = co.tenant_id and cur.id = co.default_currency where st.tenant_id = $1 limit 1",
    )
    .bind(tenant)
    .fetch_optional(&mut **tx)
    .await?)
}

/// A course's price where courses are sold (more than nothing), and its currency's code.
pub(crate) async fn price(ctx: &Ctx, tx: &mut Tx, course: &Doc) -> Result<Option<(Decimal, Option<String>)>> {
    if !installed(ctx) || d(course, "lpay_price") <= Decimal::ZERO {
        return Ok(None);
    }
    Ok(Some((d(course, "lpay_price"), currency(tx, ctx.tenant).await?)))
}

/// A price as pages show it.
pub(crate) fn tag(amount: Decimal, currency: Option<String>) -> Value {
    json!({"amount": money(amount), "currency": currency})
}

/// Prices of these courses, for catalog cards (none where courses are not sold, or are free).
pub(crate) async fn prices(ctx: &Ctx, tx: &mut Tx, courses: &[Uuid]) -> Result<Vec<(Uuid, Value)>> {
    if !installed(ctx) || courses.is_empty() {
        return Ok(vec![]);
    }
    let rows: Vec<(Uuid, Decimal)> = sqlx::query_as("select id, lpay_price from e_lms_course where tenant_id = $1 and id = any($2) and lpay_price > 0")
        .bind(ctx.tenant)
        .bind(courses)
        .fetch_all(&mut **tx)
        .await?;
    if rows.is_empty() {
        return Ok(vec![]);
    }
    let code = currency(tx, ctx.tenant).await?;
    Ok(rows.into_iter().map(|(id, p)| (id, tag(p, code.clone()))).collect())
}

/// The learner's latest purchase of the course that is not cancelled: (id, status).
async fn latest(tx: &mut Tx, tenant: Uuid, course: Uuid, learner: &str) -> Result<Option<(Uuid, String)>> {
    Ok(sqlx::query_as(
        "select id, status from e_lms_course_purchase where tenant_id = $1 and course = $2 and lower(learner) = lower($3) and status <> 'Cancelled' \
         order by created_at desc, id desc limit 1",
    )
    .bind(tenant)
    .bind(course)
    .bind(learner)
    .fetch_optional(&mut **tx)
    .await?)
}

/// Refuses someone's own enrolment in a paid course they have not paid for.
pub(crate) async fn check_paid(ctx: &Ctx, tx: &mut Tx, course: &Doc, learner: &str) -> Result<()> {
    if price(ctx, tx, course).await?.is_none() {
        return Ok(());
    }
    match latest(tx, ctx.tenant, doc_id(course)?, learner).await? {
        Some((_, st)) if st == PAID => Ok(()),
        _ => Err(Error::field("course", "this course is paid: buy it on its page")),
    }
}

/// What the course page shows of a sale: the price, the learner's purchase while it matters, and
/// whether they can buy it now (`blocked`: what else keeps them from enrolling).
pub(crate) async fn offer(k: &Kernel, ctx: &Ctx, tx: &mut Tx, course: &Doc, enrolled: Option<&Doc>, blocked: bool) -> Result<Option<Value>> {
    let Some((amount, currency)) = price(ctx, tx, course).await? else { return Ok(None) };
    let cid = doc_id(course)?;
    let mut purchase = Value::Null;
    let mut paid = false;
    if let Some((p, status)) = latest(tx, ctx.tenant, cid, &ctx.email).await? {
        paid = status == PAID;
        if status != REFUNDING || enrolled.is_none_or(|e| s(e, "status") == Some(WITHDRAWN)) {
            purchase = view(k, ctx, tx, p).await?;
        }
    }
    let live = enrolled.is_some_and(|e| s(e, "status") != Some(WITHDRAWN));
    let method = terms(tx, ctx.tenant).await?.method;
    Ok(Some(json!({
        "price": tag(amount, currency), "purchase": purchase, "paid": paid,
        "can_buy": !live && !paid && !blocked && method.is_some() && purchase.get("status").and_then(Value::as_str) != Some(AWAITING),
    })))
}

/// A purchase as its learner sees it, with the way to pay while it is due.
async fn view(k: &Kernel, ctx: &Ctx, tx: &mut Tx, purchase: Uuid) -> Result<Value> {
    let p = load(k, ctx, tx, "LmsCoursePurchase", purchase).await?;
    let request = match id(&p, "payment_request") {
        Some(pr) => sqlx::query_as::<_, (String, Option<String>)>("select status, payment_url from e_payment_request where tenant_id = $1 and id = $2")
            .bind(ctx.tenant)
            .bind(pr)
            .fetch_optional(&mut **tx)
            .await?
            .map(|(status, url)| json!({"status": status, "payment_url": if s(&p, "status") == Some(AWAITING) { url } else { None }})),
        None => None,
    };
    Ok(json!({
        "id": purchase, "number": p.get("number"), "status": p.get("status"), "amount": p.get("amount"), "paid_at": p.get("paid_at"),
        "payment": request,
    }))
}

/// On courses: the Buy action.
pub struct CourseHook;

#[async_trait]
impl DocHook for CourseHook {
    async fn action(&self, hc: &mut HookCtx<'_>, name: &str, doc: &Doc, _params: &Value) -> Result<Option<Value>> {
        if name != "buy" {
            return Ok(None);
        }
        Ok(Some(buy(hc, doc).await?))
    }
}

/// Invoices the learner for the course and asks for the invoice through the store; a purchase still
/// awaiting payment is asked for again rather than invoiced twice.
async fn buy(hc: &mut HookCtx<'_>, course: &Doc) -> Result<Value> {
    let ctx = hc.ctx;
    if ctx.user.is_nil() || !util::is_member(ctx) {
        return Err(Error::Forbidden("sign in with a Learn role to buy a course".into()));
    }
    if price(ctx, hc.tx, course).await?.is_none() {
        return Err(Error::invalid("this course is free: enrol in it"));
    }
    let learner = ctx.email.to_lowercase();
    let cid = doc_id(course)?;
    if !crate::catalog::is_published(course) || !b(course, "self_enrollment") {
        return Err(Error::invalid("this course is not on sale: ask an instructor"));
    }
    let missing = enrollment::missing_prerequisites(ctx, hc.tx, course, &learner).await?;
    if !missing.is_empty() {
        return Err(Error::invalid(format!("complete these courses first: {}", missing.join(", "))));
    }
    let enrolled: Option<String> = sqlx::query_scalar(
        "select status from e_lms_enrollment where tenant_id = $1 and course = $2 and learner = $3 order by coalesce(cycle, 1) desc limit 1",
    )
    .bind(ctx.tenant)
    .bind(cid)
    .bind(&learner)
    .fetch_optional(&mut **hc.tx)
    .await?;
    if enrolled.as_deref().is_some_and(|st| st != WITHDRAWN) {
        return Err(Error::invalid("you are enrolled in this course already"));
    }
    let t = terms(hc.tx, ctx.tenant).await?;
    let method = t.method.clone().ok_or_else(|| Error::invalid("courses cannot be bought yet: set the course payment method in Learn Settings"))?;
    let sctx = sys(ctx);
    let purchase = match latest(hc.tx, ctx.tenant, cid, &learner).await? {
        Some((_, st)) if st == PAID => return Err(Error::invalid("you have bought this course: enrol again from its page")),
        Some((p, st)) if st == AWAITING => {
            let doc = hc.kernel.get_tx(&sctx, hc.tx, "LmsCoursePurchase", p, true).await?;
            let open: Option<String> = match id(&doc, "payment_request") {
                Some(pr) => {
                    sqlx::query_scalar("select status from e_payment_request where tenant_id = $1 and id = $2")
                        .bind(ctx.tenant)
                        .bind(pr)
                        .fetch_optional(&mut **hc.tx)
                        .await?
                }
                None => None,
            };
            if open.as_deref() != Some("Initiated") {
                // The request lapsed or failed: the same invoice is asked for again.
                let invoice = id(&doc, "sales_invoice").ok_or_else(|| Error::internal("a purchase without its invoice"))?;
                let inv = get(hc.kernel, &sctx, hc.tx, "SalesInvoice", invoice).await?;
                ask(hc, &inv, &method, p, &learner).await?;
            }
            p
        }
        _ => {
            let name = util::user(hc.tx, ctx.tenant, &learner).await?.map(|u| u.name()).unwrap_or_else(|| learner.clone());
            let mut input = Doc::new();
            set(&mut input, "course", cid.to_string());
            set(&mut input, "learner", learner.clone());
            set(&mut input, "learner_name", name.clone());
            set(&mut input, "status", AWAITING);
            let p = hc.kernel.insert_tx(&sctx, hc.tx, "LmsCoursePurchase", input).await?;
            let p = doc_id(&p)?;
            let inv = invoice(hc, &t, course, &learner, &name).await?;
            let mut v = Doc::new();
            set(&mut v, "sales_invoice", s(&inv, "id").unwrap_or_default());
            set(&mut v, "amount", money(d(&inv, "grand_total")));
            hc.kernel.set_values_tx(&sctx, hc.tx, "LmsCoursePurchase", p, v).await?;
            ask(hc, &inv, &method, p, &learner).await?;
            p
        }
    };
    Ok(json!({"purchase": view(hc.kernel, ctx, hc.tx, purchase).await?}))
}

/// A submitted invoice of the course for the learner.
async fn invoice(hc: &mut HookCtx<'_>, t: &Terms, course: &Doc, learner: &str, name: &str) -> Result<Doc> {
    let sctx = sys(hc.ctx);
    let company = t.company.ok_or_else(|| Error::invalid("courses cannot be bought yet: set the selling company in Learn Settings"))?;
    let item = id(course, "lpay_item").ok_or_else(|| Error::invalid("this course has a price but no item to invoice it as"))?;
    let currency: Uuid = sqlx::query_scalar("select default_currency from e_company where tenant_id = $1 and id = $2")
        .bind(hc.ctx.tenant)
        .bind(company)
        .fetch_optional(&mut **hc.tx)
        .await?
        .flatten()
        .ok_or_else(|| Error::invalid("the selling company in Learn Settings has no currency"))?;
    let customer = customer_for(hc, learner, name).await?;
    let mut input = Doc::new();
    set(&mut input, "company", company.to_string());
    set(&mut input, "customer", customer.to_string());
    set(&mut input, "currency", currency.to_string());
    set(
        &mut input,
        "items",
        json!([{"item": item.to_string(), "qty": "1", "price_list_rate": money(d(course, "lpay_price")), "discount_percent": "0",
                "description": format!("Course: {}", s(course, "title").unwrap_or_default())}]),
    );
    set(&mut input, "remarks", format!("Course {} for {learner}", s(course, "title").unwrap_or_default()));
    let draft = hc.kernel.insert_tx(&sctx, hc.tx, "SalesInvoice", input).await?;
    let inv = doc_id(&draft)?;
    hc.kernel.submit_tx(&sctx, hc.tx, "SalesInvoice", inv, None).await?;
    get(hc.kernel, &sctx, hc.tx, "SalesInvoice", inv).await
}

/// The learner's customer: an enabled one with their email, or a new individual customer.
async fn customer_for(hc: &mut HookCtx<'_>, email: &str, name: &str) -> Result<Uuid> {
    let found: Option<Uuid> =
        sqlx::query_scalar("select id from e_customer where tenant_id = $1 and lower(email) = lower($2) and not disabled order by created_at, id limit 1")
            .bind(hc.ctx.tenant)
            .bind(email)
            .fetch_optional(&mut **hc.tx)
            .await?;
    if let Some(c) = found {
        return Ok(c);
    }
    let mut input = Doc::new();
    set(&mut input, "customer_name", name);
    set(&mut input, "customer_type", "Individual");
    set(&mut input, "email", email);
    doc_id(&hc.kernel.insert_tx(&sys(hc.ctx), hc.tx, "Customer", input).await?)
}

/// Asks for what is outstanding on the purchase's invoice through the store.
async fn ask(hc: &mut HookCtx<'_>, inv: &Doc, method: &str, purchase: Uuid, learner: &str) -> Result<()> {
    let sctx = sys(hc.ctx);
    let request = ferrum_app_commerce::request_invoice_payment(hc.kernel, &sctx, hc.tx, inv, method, None, Some(learner.to_string()))
        .await?
        .ok_or_else(|| Error::invalid("nothing is outstanding on this purchase's invoice"))?;
    let mut v = Doc::new();
    set(&mut v, "payment_request", s(&request, "id").unwrap_or_default());
    hc.kernel.set_values_tx(&sctx, hc.tx, "LmsCoursePurchase", purchase, v).await
}

/// On payment entries: a course paid enrols its learner (again, if they had withdrawn).
pub struct PaidHook;

#[async_trait]
impl DocHook for PaidHook {
    async fn on_submit(&self, hc: &mut HookCtx<'_>, doc: &Doc) -> Result<()> {
        let Some(pr) = id(doc, "commerce_payment_request") else { return Ok(()) };
        let found: Option<(Uuid, Uuid, String)> = sqlx::query_as(
            "select id, course, learner from e_lms_course_purchase where tenant_id = $1 and payment_request = $2 and status = 'Awaiting Payment' for update",
        )
        .bind(hc.ctx.tenant)
        .bind(pr)
        .fetch_optional(&mut **hc.tx)
        .await?;
        let Some((purchase, course, learner)) = found else { return Ok(()) };
        let engine = elevated(hc.ctx);
        let sctx = sys(hc.ctx);
        let mut v = Doc::new();
        set(&mut v, "status", PAID);
        set(&mut v, "paid_at", util::ts(util::now()));
        hc.kernel.set_values_tx(&sctx, hc.tx, "LmsCoursePurchase", purchase, v).await?;
        let enrolment = match enrollment::try_enroll(hc.kernel, &engine, hc.tx, course, &learner, None).await? {
            Outcome::Enrolled(e) => Some(doc_id(&e)?),
            Outcome::Already(e) => {
                let eid: Uuid = e["id"].as_str().and_then(|v| v.parse().ok()).ok_or_else(|| Error::internal("enrolment without id"))?;
                if e["status"] == json!(WITHDRAWN) {
                    let mut back = Doc::new();
                    set(&mut back, "status", ACTIVE);
                    hc.kernel.update_tx(&engine, hc.tx, "LmsEnrollment", eid, back, None).await?;
                    let fresh = util::load_locked(hc.kernel, hc.ctx, hc.tx, "LmsEnrollment", eid).await?;
                    enrollment::refresh(hc.kernel, hc.ctx, hc.tx, &fresh, None).await?;
                }
                Some(eid)
            }
            Outcome::Refused(why) => {
                // Paid but not enrolled (the learner's account was closed meanwhile): a person settles it.
                tracing::warn!(tenant = %hc.ctx.tenant, %purchase, "a paid course could not enrol its learner: {why}");
                None
            }
        };
        if let Some(e) = enrolment {
            let mut v = Doc::new();
            set(&mut v, "enrollment", e.to_string());
            hc.kernel.set_values_tx(&sctx, hc.tx, "LmsCoursePurchase", purchase, v).await?;
        }
        Ok(())
    }
}

/// On enrolments: withdrawing from a paid course within the refund period gives the price back.
pub struct EnrollmentHook;

#[async_trait]
impl DocHook for EnrollmentHook {
    async fn after_save(&self, hc: &mut HookCtx<'_>, doc: &Doc, old: Option<&Doc>) -> Result<()> {
        let Some(old) = old else { return Ok(()) };
        if s(doc, "status") != Some(WITHDRAWN) || s(old, "status") != Some(ACTIVE) {
            return Ok(());
        }
        let (Some(course), Some(learner)) = (id(doc, "course"), s(doc, "learner")) else { return Ok(()) };
        let Some((purchase, status)) = latest(hc.tx, hc.ctx.tenant, course, learner).await? else { return Ok(()) };
        if status != PAID {
            return Ok(());
        }
        let sctx = sys(hc.ctx);
        let p = hc.kernel.get_tx(&sctx, hc.tx, "LmsCoursePurchase", purchase, true).await?;
        let t = terms(hc.tx, hc.ctx.tenant).await?;
        let paid_at = util::time(&p, "paid_at").ok_or_else(|| Error::internal("a paid purchase without its time"))?;
        if !refundable(paid_at, util::now(), t.refund_days) {
            return Ok(());
        }
        let (Some(pr), Some(invoice)) = (id(&p, "payment_request"), id(&p, "sales_invoice")) else { return Ok(()) };
        let inv = get(hc.kernel, &sctx, hc.tx, "SalesInvoice", invoice).await?;
        let note = credit(hc, &inv).await?;
        let paid = d(&p, "amount");
        let owed_back = (-d(&note, "outstanding_amount")).max(Decimal::ZERO).min(paid);
        let mut v = Doc::new();
        set(&mut v, "status", REFUNDING);
        set(&mut v, "credit_note", s(&note, "id").unwrap_or_default());
        if owed_back > Decimal::ZERO {
            let who = s(doc, "learner_name").unwrap_or(learner);
            let why = format!("{who} withdrew from the course within the refund period");
            let refund = ferrum_app_commerce::refund_payment(hc.kernel, &sctx, hc.tx, pr, owed_back, id(&note, "id"), &why).await?;
            set(&mut v, "refund", s(&refund, "id").unwrap_or_default());
        }
        hc.kernel.set_values_tx(&sctx, hc.tx, "LmsCoursePurchase", purchase, v).await
    }
}

/// Whether a course paid at `paid_at` is refunded when its learner withdraws at `now`: within
/// `days` of paying (0: never).
pub fn refundable(paid_at: DateTime<Utc>, now: DateTime<Utc>, days: i64) -> bool {
    days > 0 && now < paid_at + Duration::days(days)
}

/// A submitted credit note for the whole of `inv`.
async fn credit(hc: &mut HookCtx<'_>, inv: &Doc) -> Result<Doc> {
    let sctx = sys(hc.ctx);
    let mut input = Doc::new();
    for f in ["company", "customer", "currency", "conversion_rate", "price_list", "tax_template", "customer_address", "cost_center"] {
        if let Some(v) = inv.get(f).filter(|v| !v.is_null()) {
            input.insert(f.to_string(), v.clone());
        }
    }
    set(&mut input, "is_return", true);
    set(&mut input, "return_against", s(inv, "id").unwrap_or_default());
    set(&mut input, "remarks", format!("Course refunded: credit of {}", s(inv, "number").unwrap_or_default()));
    let lines: Vec<Value> = rows(inv, "items")
        .iter()
        .map(|l| {
            json!({"item": l.get("item"), "qty": money(-d(l, "qty")), "price_list_rate": l.get("price_list_rate"), "discount_percent": l.get("discount_percent"),
                   "description": l.get("description")})
        })
        .collect();
    set(&mut input, "items", Value::Array(lines));
    let draft = hc.kernel.insert_tx(&sctx, hc.tx, "SalesInvoice", input).await?;
    let note = doc_id(&draft)?;
    hc.kernel.submit_tx(&sctx, hc.tx, "SalesInvoice", note, None).await?;
    get(hc.kernel, &sctx, hc.tx, "SalesInvoice", note).await
}

/// Purchases whose payment request lapsed, failed or was cancelled a day ago or more, unpaid: they
/// are cancelled, and so is their invoice while nothing was paid on it.
pub(crate) async fn cancel_lapsed(k: &Kernel, ctx: &Ctx) -> Result<usize> {
    if !installed(ctx) {
        return Ok(0);
    }
    let mut tx = k.db.tenant_tx(ctx.tenant).await?;
    let lapsed: Vec<(Uuid, Option<Uuid>)> = sqlx::query_as(
        "select p.id, p.sales_invoice from e_lms_course_purchase p join e_payment_request r on r.tenant_id = p.tenant_id and r.id = p.payment_request \
         where p.tenant_id = $1 and p.status = 'Awaiting Payment' and r.status in ('Expired', 'Failed', 'Cancelled') \
         and r.updated_at < now() - interval '1 day' order by p.id limit 200",
    )
    .bind(ctx.tenant)
    .fetch_all(&mut *tx)
    .await?;
    let engine = elevated(ctx);
    for (purchase, invoice) in &lapsed {
        if let Some(inv) = invoice {
            let row: Option<(i16, Decimal, Decimal)> =
                sqlx::query_as("select docstatus, outstanding_amount, grand_total from e_sales_invoice where tenant_id = $1 and id = $2")
                    .bind(ctx.tenant)
                    .bind(inv)
                    .fetch_optional(&mut *tx)
                    .await?;
            if let Some((1, outstanding, total)) = row
                && outstanding == total
            {
                k.cancel_tx(&engine, &mut tx, "SalesInvoice", *inv, None).await?;
            }
        }
        let mut v = Doc::new();
        set(&mut v, "status", CANCELLED);
        k.set_values_tx(ctx, &mut tx, "LmsCoursePurchase", *purchase, v).await?;
    }
    tx.commit().await?;
    Ok(lapsed.len())
}

#[cfg(test)]
mod tests {
    use super::refundable;
    use chrono::{DateTime, Utc};

    #[test]
    fn refunds_only_within_the_period() {
        let at = |t: &str| DateTime::parse_from_rfc3339(t).unwrap().with_timezone(&Utc);
        let paid = at("2026-11-01T10:00:00Z");
        assert!(refundable(paid, at("2026-11-15T09:59:59Z"), 14));
        assert!(!refundable(paid, at("2026-11-15T10:00:00Z"), 14), "fourteen days to the second");
        assert!(!refundable(paid, at("2026-11-01T10:00:01Z"), 0), "no refund period");
    }
}
