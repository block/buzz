//! Workload substrates: the side-effect half of workload reconciliation.
//!
//! The durable [`crate::WorkloadLedger`] is pure bookkeeping — it records what
//! the owner asked for and what the node last observed. A substrate is the
//! thing that *does* something about it: nothing at all ([`InertSubstrate`]),
//! a supervised child process on the node ([`process::ProcessSubstrate`]), or
//! a Docker container running the agent body image
//! ([`docker::DockerSubstrate`]).

use std::fmt;

use async_trait::async_trait;
use buzz_core::execution::{SafeErrorCode, WorkloadId, WorkloadSpec};

pub mod docker;
pub(crate) mod env;
pub mod process;

/// Default inactivity budget handed to every workload body as
/// `BUZZ_ACP_EXIT_AFTER_INACTIVITY` — the I5 opt-in of
/// docs/remote-agents.md §Auto-Stop. Mirrors the Kubernetes binding's
/// `inactivity_seconds` schema default
/// (`crates/buzz-backend-kubernetes/src/config.rs`): remote bodies opt in,
/// and a node is remote by definition. `0` is a legal, blessed "no
/// inactivity bound" and is never rejected.
pub const DEFAULT_INACTIVITY_SECONDS: u64 = 7200;

/// A body exit observed by a substrate.
///
/// Substrates report only exits they did not cause themselves: a body that
/// exits on its own was finished, not killed, and the controller records the
/// outcome in the ledger without ever respawning it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkloadExit {
    /// Owner scope the workload was deployed under.
    pub owner: String,
    /// Workload whose body exited.
    pub workload_id: WorkloadId,
    /// Whether the body exited with a success status.
    pub clean: bool,
}

/// Failure produced by a substrate action.
///
/// The safe classification travels in the receipt; the diagnostic message is
/// node-local only — it is logged on the node and never crosses the wire.
#[derive(Debug, Clone)]
pub struct SubstrateError {
    /// Safe classification reported in the terminal receipt.
    pub code: SafeErrorCode,
    /// Node-local diagnostic. Logged on the node, never sent in receipts.
    pub message: String,
}

impl SubstrateError {
    /// Build a substrate failure from a safe code and a node-local message.
    pub fn new(code: SafeErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for SubstrateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:?}: {}", self.code, self.message)
    }
}

impl std::error::Error for SubstrateError {}

/// The seam between the durable ledger and whatever actually runs workloads.
///
/// The [`crate::WorkloadLedger`] decides whether a command is admissible and
/// what receipt it earns; the substrate is then asked to make local reality
/// match. Substrate state is process-local — a restarted node rebuilds it from
/// the persisted ledger, except one-time launch keys, which are deliberately
/// lost (see the key contract below).
///
/// Contract every substrate must honor:
/// - `deploy` receives the full [`WorkloadSpec`] **including** the one-time
///   private key and must converge to a single live body per workload —
///   replace an existing body, never run two.
/// - `start` and `restart` receive the ledger's durable, key-stripped spec;
///   launch material must come from substrate-local memory and both fail
///   closed when it is gone.
/// - `stop` and `remove` are idempotent: stopping a body that is not running
///   and removing a workload the substrate does not know both succeed.
/// - the private key is a one-time launch handoff: it must never be persisted,
///   logged, or echoed in errors or receipts.
/// - a body that exits on its own was finished, not killed. The substrate
///   reports it through its exit channel and must never resurrect it.
///
/// Every method is async and must be callable without any controller-wide
/// lock held: the [`crate::ExecutionController`] serializes same-workload
/// commands with per-workload locks and releases its state mutex across every
/// substrate await, so a bounded SIGTERM→SIGKILL wait in one workload never
/// stalls commands for another.
#[async_trait]
pub trait Substrate: fmt::Debug + Send + Sync {
    /// Deploy (or replace) the body for a workload. The spec still carries
    /// the one-time launch key.
    async fn deploy(&self, owner: &str, workload: &WorkloadSpec) -> Result<(), SubstrateError>;

    /// Start the body for a previously deployed workload from the durable,
    /// key-stripped spec plus substrate-held launch material.
    async fn start(&self, owner: &str, workload: &WorkloadSpec) -> Result<(), SubstrateError>;

    /// Stop the body for a workload. Succeeds when nothing is running.
    async fn stop(&self, owner: &str, workload_id: &WorkloadId) -> Result<(), SubstrateError>;

    /// Restart the body for a previously deployed workload: stop whatever is
    /// running, then respawn from substrate-held launch material.
    async fn restart(&self, owner: &str, workload: &WorkloadSpec) -> Result<(), SubstrateError>;

    /// Stop the body, discard launch material, and clear node-local scratch
    /// state for a workload. Succeeds when the substrate holds nothing.
    async fn remove(&self, owner: &str, workload_id: &WorkloadId) -> Result<(), SubstrateError>;
}

/// Substrate that accepts every operation and launches nothing.
///
/// A receipt issued through this substrate proves ledger reconciliation —
/// idempotency, sequencing, removal tombstones — not that an agent process is
/// running or able to answer. It remains the library default so protocol and
/// reconciliation behavior can be exercised without spawning processes, and is
/// selectable on the binary via `--substrate inert`.
#[derive(Debug, Default, Clone, Copy)]
pub struct InertSubstrate;

#[async_trait]
impl Substrate for InertSubstrate {
    async fn deploy(&self, _owner: &str, _workload: &WorkloadSpec) -> Result<(), SubstrateError> {
        Ok(())
    }

    async fn start(&self, _owner: &str, _workload: &WorkloadSpec) -> Result<(), SubstrateError> {
        Ok(())
    }

    async fn stop(&self, _owner: &str, _workload_id: &WorkloadId) -> Result<(), SubstrateError> {
        Ok(())
    }

    async fn restart(&self, _owner: &str, _workload: &WorkloadSpec) -> Result<(), SubstrateError> {
        Ok(())
    }

    async fn remove(&self, _owner: &str, _workload_id: &WorkloadId) -> Result<(), SubstrateError> {
        Ok(())
    }
}
