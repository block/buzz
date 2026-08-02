mod activation;
mod archive;
pub(crate) mod commands;
mod installer;
mod launcher;
mod manifest;
mod store;
mod verifier;

pub(crate) use activation::{
    activate_receipt, deactivate, load_active_receipt, recover_activation, rollback,
    rollback_available, uninstall_active,
};
pub(crate) use archive::inspect_and_extract;
pub(crate) use installer::install_current;
pub use launcher::launcher_main;
pub(crate) use manifest::{builtin_manifest, builtin_manifest_sha256};
pub(crate) use store::{load_verified_receipt, InstalledReceipt};
pub(crate) use verifier::fetch_verified_artifact_to;
