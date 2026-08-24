//! Compile-time example for the SDK-only managed-workflow owner API.

use buzz_sdk::{build_workflow_owner_command, WorkflowOwnerOperation};
use uuid::Uuid;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _builder = build_workflow_owner_command(
        Uuid::new_v4(),
        &"a".repeat(64),
        Uuid::new_v4(),
        &"b".repeat(64),
        WorkflowOwnerOperation::Disable,
        None,
    )?;
    Ok(())
}
