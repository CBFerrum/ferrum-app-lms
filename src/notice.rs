//! Telling people: an in-app notification for every recipient who is an active user, and — when
//! Learn Settings allow it — an email queued in the same transaction: as a `Communication` when the
//! workspace has the Mail app (which delivers it after commit, with its retries and threading),
//! otherwise through the server's own mailer.

use crate::util::{self, html_escape, sys};
use ferrum_kernel::{Ctx, Doc, Error, Kernel, Result, Tx, notify};
use serde_json::json;
use uuid::Uuid;

pub struct Notice<'a> {
    pub title: String,
    pub body: String,
    /// The record the notification opens.
    pub entity: &'a str,
    pub record: Uuid,
}

/// Whether the workspace has the Mail app.
pub fn has_mail(ctx: &Ctx) -> bool {
    ctx.schema.packages.iter().any(|p| p == "comms") && ctx.schema.entity("Communication").is_some()
}

/// Sends `n` to the given addresses (unknown or inactive ones are skipped); returns how many
/// people were told.
pub async fn send(k: &Kernel, ctx: &Ctx, tx: &mut Tx, to: &[String], n: &Notice<'_>) -> Result<usize> {
    let mut emails: Vec<String> = to.iter().map(|e| e.trim().to_lowercase()).filter(|e| !e.is_empty()).collect();
    emails.sort();
    emails.dedup();
    let people = util::users(tx, ctx.tenant, &emails).await?;
    if people.is_empty() {
        return Ok(0);
    }
    let title = util::cut(&n.title, 200);
    for p in &people {
        notify::send(tx, ctx.tenant, p.id, &title, Some(&n.body), Some(n.entity), Some(n.record)).await?;
    }
    let mail_app = has_mail(ctx);
    if (mail_app || k.mailer.is_some()) && util::settings(tx, ctx.tenant).await?.email {
        let engine = sys(ctx);
        let html = format!("<p>{}</p>", html_escape(&n.body).replace('\n', "<br>"));
        for p in &people {
            if !mail_app {
                let subject: String = title.replace(['\r', '\n'], " ").chars().take(300).collect();
                match k.queue_mail(tx, ctx.tenant, &p.email, &subject, &html).await {
                    Ok(()) => {}
                    // An address the mailer refuses costs that person the email, not the change.
                    Err(Error::Validation(e)) => tracing::warn!(to = %p.email, "learn: email not queued: {e:?}"),
                    Err(e) => return Err(e),
                }
                continue;
            }
            let mut mail = Doc::new();
            for (field, value) in [
                ("channel", json!("Email")),
                ("direction", json!("Sent")),
                ("recipients", json!(p.email)),
                ("subject", json!(title)),
                ("content", json!(html)),
                ("text_content", json!(n.body)),
                ("reference_entity", json!(n.entity)),
                ("reference_id", json!(n.record.to_string())),
            ] {
                mail.insert(field.into(), value);
            }
            k.insert_tx(&engine, tx, "Communication", mail).await?;
        }
    }
    Ok(people.len())
}
