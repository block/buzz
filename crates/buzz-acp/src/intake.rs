//! Typed admission results shared by relay and local forward ingress.
use crate::relay::BuzzEvent;
use tokio::sync::oneshot;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnqueueResult {
    Queued,
    /// Policy `DedupMode::Drop` discarded the in-flight channel event.
    Drop,
    /// Occupied but not queued (filter / author / self). Ack is still accepted.
    Ignored,
    /// Queue never happened (main loop gone, channel closed, oneshot dropped).
    /// Must not be acked — the gateway treats silence as delivery-unknown.
    #[cfg(unix)]
    InfraFailure,
}

#[derive(Debug)]
pub struct ForwardWork {
    pub event: BuzzEvent,
    pub reply: oneshot::Sender<EnqueueResult>,
}
