//! People outside the workspace who sign themselves up on a public course page (LM-60).
//!
//! The workspace turns public sign-up on for the **Online Learner** portal role, passing `course`
//! (`PUT /api/admin/roles/Online Learner/signup {passes: ["course"]}`). A course page then posts the
//! platform's `POST /api/auth/code/sign-up {tenant, role, name, email, passes: {course: <slug>}}`;
//! once the mailed code is confirmed the platform raises `ferrum.portal.signed_up`, and here the new
//! person is enrolled in a free course, or a paid one is bought for them (its invoice and payment
//! request, as the Buy button makes them; the payment then enrols them). The course passed is checked
//! as a self-enrolment would be: published, public, open to self-enrolment, prerequisites met.
//! Delivery is at least once: an enrolment is one per learner and course, and buying again reuses the
//! purchase still awaiting payment, so a repeated event changes nothing.

use crate::enrollment::{self, Outcome};
use crate::util::{self, is_member};
use async_trait::async_trait;
use ferrum_kernel::outbox::Event;
use ferrum_kernel::portal_signup::SIGNED_UP;
use ferrum_kernel::{Ctx, Error, EventSink, Kernel, Result};
use serde_json::{Value, json};
use uuid::Uuid;

pub struct SignupSink;

#[async_trait]
impl EventSink for SignupSink {
    async fn on_event(&self, k: &Kernel, ev: &Event) -> Result<()> {
        if ev.topic != SIGNED_UP {
            return Ok(());
        }
        let course = match &ev.payload["passed"]["course"] {
            Value::String(v) => v.trim().to_string(),
            Value::Number(n) => n.to_string(),
            _ => return Ok(()),
        };
        let Some(user) = ev.payload["user"].as_str().and_then(|v| v.parse::<Uuid>().ok()) else { return Ok(()) };
        let schema = k.registry.schema(ev.tenant_id).await?;
        if !schema.packages.iter().any(|p| p == util::PACKAGE) || course.is_empty() {
            return Ok(());
        }
        let ctx = match k.load_ctx(ev.tenant_id, user, None, "lms-signup".into()).await {
            Ok(c) => c,
            // Gone or switched off since: nothing to enrol.
            Err(Error::NotFound(_) | Error::Unauthorized(_) | Error::Forbidden(_)) => return Ok(()),
            Err(e) => return Err(e),
        };
        if !is_member(&ctx) {
            return Ok(());
        }
        match take(k, &ctx, &course).await {
            Err(e) if e.is_retryable() => Err(e),
            Err(e) => {
                tracing::warn!(tenant = %ctx.tenant, %user, course, "a sign-up's course was not taken: {e}");
                Ok(())
            }
            Ok(_) => Ok(()),
        }
    }
}

/// Enrols the signed-up person in the course they came from (by slug or id), or buys it for them
/// when it is paid: what happened, `{enrolled|purchase|refused}`.
pub async fn take(k: &Kernel, ctx: &Ctx, course: &str) -> Result<Value> {
    let mut tx = k.db.tenant_tx(ctx.tenant).await?;
    let found: Option<Uuid> = sqlx::query_scalar(
        "select id from e_lms_course where tenant_id = $1 and (slug = $2 or id::text = $2) and status = 'Published' and visibility = 'Public' \
         and coalesce(self_enrollment, false) limit 1",
    )
    .bind(ctx.tenant)
    .bind(course)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(cid) = found else { return Ok(json!({"refused": "no public course open to sign-up has this name"})) };
    let doc = util::load(k, ctx, &mut tx, "LmsCourse", cid).await?;
    if crate::payments::price(ctx, &mut tx, &doc).await?.is_some() {
        tx.commit().await?;
        // The Buy button's own path: the invoice and the payment request; the payment enrols them.
        return k.run_action(ctx, "LmsCourse", cid, "buy", json!({})).await;
    }
    let out = match enrollment::try_enroll(k, ctx, &mut tx, cid, &ctx.email.to_lowercase(), None).await? {
        Outcome::Enrolled(e) => json!({"enrolled": e.get("id")}),
        Outcome::Already(e) => json!({"enrolled": e["id"]}),
        Outcome::Refused(why) => json!({"refused": why}),
    };
    tx.commit().await?;
    Ok(out)
}

/// What a public course page says about signing up for it: open when the workspace takes public
/// sign-ups as Online Learner passing `course`, and the course takes self-enrolment. The page posts
/// `role` and `passes` to the platform's sign-up as they are.
pub async fn offer(tx: &mut ferrum_kernel::Tx, tenant: Uuid, course: &ferrum_kernel::Doc) -> Result<Value> {
    let setting: Option<Value> = sqlx::query_scalar("select public_signup from role where tenant_id = $1 and name = $2 and portal")
        .bind(tenant)
        .bind(util::ONLINE_LEARNER)
        .fetch_optional(&mut **tx)
        .await?
        .flatten();
    let takes_course = setting.as_ref().and_then(|s| s["passes"].as_array()).is_some_and(|p| p.iter().any(|v| v == "course"));
    if !takes_course || !util::b(course, "self_enrollment") {
        return Ok(Value::Null);
    }
    Ok(json!({"role": util::ONLINE_LEARNER, "passes": {"course": course.get("slug")}}))
}
