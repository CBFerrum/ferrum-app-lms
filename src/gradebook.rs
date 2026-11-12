//! The gradebook (LM-45).

use ferrum_kernel::{Ctx, Kernel, Result, Tx};
use uuid::Uuid;

/// A grade of the enrolment changed (a quiz attempt or a submission graded).
pub async fn changed(_k: &Kernel, _ctx: &Ctx, _tx: &mut Tx, _enrollment: Uuid) -> Result<()> {
    Ok(())
}
