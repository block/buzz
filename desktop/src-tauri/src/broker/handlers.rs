//! The three `agents.*` capability handlers.
//!
//! Each handler translates a validated broker request into a call on
//! [`AgentService`] and translates the result back into a
//! [`HandlerOutcome`]. They contain no policy (that is `agents_policy`), no
//! idempotency logic (that is `store`), and no transport concerns.
//!
//! # Why a trait instead of calling the commands directly
//!
//! The agent mutations live behind Tauri command signatures that need an
//! `AppHandle` and a `State`. Depending on those directly here would make every
//! handler untestable without booting an app, and would leave the capability
//! layer entangled with the desktop runtime. [`AgentService`] is the seam: the
//! handlers are pure and tested against a fake, and the Tauri-backed
//! implementation stays thin enough to read in one screen.

use buzz_sdk_pkg::broker::{
    AgentsCreateArgs, AgentsCreateOutcome, AgentsDeleteArgs, AgentsDeleteOutcome, AgentsUpdateArgs,
    AgentsUpdateOutcome, BrokerError, BrokerErrorCode, BrokerRequest, Capability, CapabilityArgs,
    CapabilityOutcome,
};

use super::pipeline::{
    BoxFuture, CapabilityHandler, CapabilityRegistry, HandlerOutcome, VerifiedRequest,
};

/// Why a domain mutation did not succeed.
///
/// The distinction is the whole point: [`Self::Failed`] asserts that no side
/// effects persisted, while [`Self::Unknown`] asserts nothing. A service that
/// mutated and then lost track must return `Unknown` so the broker records
/// `indeterminate` instead of implying a clean failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServiceError {
    /// The operation did not take effect.
    Failed(String),
    /// Whether the operation took effect is unknown.
    Unknown(String),
}

impl ServiceError {
    fn into_outcome(self) -> HandlerOutcome {
        match self {
            Self::Failed(message) => {
                HandlerOutcome::Failed(BrokerError::new(BrokerErrorCode::CapabilityFailed, message))
            }
            Self::Unknown(message) => HandlerOutcome::Indeterminate(BrokerError::new(
                BrokerErrorCode::OutcomeUnknown,
                message,
            )),
        }
    }
}

/// The agent mutations the broker can perform on an owner's behalf.
///
/// Implementations own credential access. Nothing in this trait returns secret
/// material: a create reports the new agent's public key, never its nsec.
///
/// Async because the real implementation mints a key, writes the agent store,
/// and publishes to the relay. `Send + Sync` so a handler can be dispatched
/// from the async pipeline.
pub trait AgentService: Send + Sync {
    /// Mint an agent and attach it to the requested channel.
    fn create<'a>(
        &'a self,
        owner_pubkey: &'a str,
        args: &'a AgentsCreateArgs,
    ) -> BoxFuture<'a, Result<AgentsCreateOutcome, ServiceError>>;

    /// Patch one of the owner's agents.
    fn update<'a>(
        &'a self,
        owner_pubkey: &'a str,
        args: &'a AgentsUpdateArgs,
    ) -> BoxFuture<'a, Result<AgentsUpdateOutcome, ServiceError>>;

    /// Remove one of the owner's agents.
    fn delete<'a>(
        &'a self,
        owner_pubkey: &'a str,
        args: &'a AgentsDeleteArgs,
    ) -> BoxFuture<'a, Result<AgentsDeleteOutcome, ServiceError>>;
}

/// Dispatches the three `agents.*` capabilities to an [`AgentService`].
pub struct AgentsHandlers<S> {
    service: S,
}

impl<S: AgentService> AgentsHandlers<S> {
    /// Wrap `service`.
    pub fn new(service: S) -> Self {
        Self { service }
    }
}

/// Handler for one capability, borrowing the shared service.
struct Handler<'a, S> {
    service: &'a S,
    capability: Capability,
}

impl<S: AgentService> CapabilityHandler for Handler<'_, S> {
    fn execute<'a>(
        &'a self,
        request: &'a VerifiedRequest,
        parsed: &'a BrokerRequest,
    ) -> BoxFuture<'a, HandlerOutcome> {
        Box::pin(async move {
            // The pipeline validated the envelope, but the args it validated are
            // the ones this handler must act on -- so it re-normalizes rather
            // than trusting a separately parsed copy.
            let args = match parsed.capability.validated() {
                Ok(args) => args,
                Err(error) => {
                    return HandlerOutcome::Failed(BrokerError::invalid_request(error.to_string()))
                }
            };

            // A capability name that disagrees with its args is a routing bug,
            // not a caller error, and must never reach a mutation.
            if args.capability() != self.capability {
                return HandlerOutcome::Failed(BrokerError::new(
                    BrokerErrorCode::Internal,
                    "broker routed a request to the wrong capability handler",
                ));
            }

            let owner = &request.owner_pubkey;
            match &args {
                CapabilityArgs::AgentsCreate(args) => {
                    match self.service.create(owner, args).await {
                        Ok(outcome) => {
                            HandlerOutcome::Succeeded(CapabilityOutcome::AgentsCreate(outcome))
                        }
                        Err(error) => error.into_outcome(),
                    }
                }
                CapabilityArgs::AgentsUpdate(args) => {
                    match self.service.update(owner, args).await {
                        Ok(outcome) => {
                            HandlerOutcome::Succeeded(CapabilityOutcome::AgentsUpdate(outcome))
                        }
                        Err(error) => error.into_outcome(),
                    }
                }
                CapabilityArgs::AgentsDelete(args) => {
                    match self.service.delete(owner, args).await {
                        Ok(outcome) => {
                            HandlerOutcome::Succeeded(CapabilityOutcome::AgentsDelete(outcome))
                        }
                        Err(error) => error.into_outcome(),
                    }
                }
            }
        })
    }
}

/// Registry over the three `agents.*` handlers.
pub struct AgentsRegistry<'a, S> {
    create: Handler<'a, S>,
    update: Handler<'a, S>,
    delete: Handler<'a, S>,
}

impl<S: AgentService> AgentsHandlers<S> {
    /// Build the registry this host offers.
    pub fn registry(&self) -> AgentsRegistry<'_, S> {
        AgentsRegistry {
            create: Handler {
                service: &self.service,
                capability: Capability::AgentsCreate,
            },
            update: Handler {
                service: &self.service,
                capability: Capability::AgentsUpdate,
            },
            delete: Handler {
                service: &self.service,
                capability: Capability::AgentsDelete,
            },
        }
    }
}

impl<S: AgentService> CapabilityRegistry for AgentsRegistry<'_, S> {
    fn handler(&self, capability: Capability) -> Option<&dyn CapabilityHandler> {
        Some(match capability {
            Capability::AgentsCreate => &self.create,
            Capability::AgentsUpdate => &self.update,
            Capability::AgentsDelete => &self.delete,
        })
    }
}

#[cfg(test)]
mod tests;
