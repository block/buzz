#![forbid(unsafe_code)]

mod acceptance;
mod api;
mod approval;
mod catalog;
mod config;
mod domain;
mod helpers;
mod lifecycle_internal;
mod model;
mod persistence;

pub use acceptance::{acceptance_actor, acceptance_engine, ACCEPTANCE_CLASSIFICATION};
pub use api::{
    acceptance_router, acceptance_router_with_gateway, router, AgentVerifier, ApiState,
    AGENT_ACTION_TOOLS,
};
pub use catalog::{bundled_catalog, catalog_from_path};
pub use config::{ActionMode, Config};
pub use domain::{ActionEngine, ActionState};
pub use model::{
    ActionError, Actor, AssigneeResolver, AuditEvent, ConfirmApprovalDraft, ConfirmWorkItem,
    CurrentUserOnlyAssigneeResolver, FindingObservation, PrepareApprovalDraft, PrepareWorkItem,
    UpdateApprovalDraft, UpdateWorkItem,
};
pub use persistence::PgActionStore;

#[cfg(test)]
mod tests;
