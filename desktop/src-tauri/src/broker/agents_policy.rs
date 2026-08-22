//! Authorization policy for the `agents.*` capabilities.
//!
//! # Why this is not broker policy
//!
//! The broker authenticates: it proves who signed a frame and that the frame
//! was sealed for this owner. Whether that signer may *manage agents* is a
//! property of the `agents.*` capability family, not of brokering in general —
//! a future capability with different scope rules must not inherit these.
//!
//! # The rule
//!
//! A requester may manage this owner's agents when the requester **is** one of
//! this owner's managed agents. That is the owner↔requester binding, and it is
//! read from the authoritative Rust roster
//! (`managed_agents::storage::load_managed_agents`), never from anything the
//! caller sent.
//!
//! There is deliberately **no channel restriction on the target** of an update
//! or delete. An owner's agent is the owner's agent regardless of which channel
//! it happens to sit in, and requiring channel co-membership would have made the
//! rule look tighter than it is while adding no real constraint: a requester
//! that can create an agent in a channel could always add its target there
//! first. Channel appears only where the operation genuinely needs it — the
//! attachment on `agents.create`.

use buzz_sdk_pkg::broker::{BrokerError, BrokerRequest, Capability};

use super::pipeline::{Authorizer, BoxFuture, VerifiedRequest};

/// The set of agents an owner controls, as far as authorization is concerned.
///
/// A trait so policy can be tested without a Tauri app handle or a real agent
/// store, and so a non-Desktop host can supply its own roster.
pub trait AgentRoster: Send + Sync {
    /// Pubkeys of every agent this owner manages.
    fn agent_pubkeys<'a>(
        &'a self,
        owner_pubkey: &'a str,
    ) -> BoxFuture<'a, Result<Vec<String>, String>>;
}

/// `agents.*` authorization backed by an [`AgentRoster`].
pub struct AgentsAuthorizer<R> {
    roster: R,
}

impl<R: AgentRoster> AgentsAuthorizer<R> {
    /// Build an authorizer over `roster`.
    pub fn new(roster: R) -> Self {
        Self { roster }
    }
}

impl<R: AgentRoster> Authorizer for AgentsAuthorizer<R> {
    fn authorize<'a>(
        &'a self,
        request: &'a VerifiedRequest,
        capability: Capability,
        _parsed: &'a BrokerRequest,
    ) -> BoxFuture<'a, Result<(), BrokerError>> {
        Box::pin(async move {
            match capability {
                Capability::AgentsCreate | Capability::AgentsUpdate | Capability::AgentsDelete => {}
            }

            // The requester must never be able to act as the owner directly:
            // that would mean a frame signed by the owner key was treated as a
            // delegated request, collapsing the distinction the broker exists
            // to maintain.
            if request
                .requester_pubkey
                .eq_ignore_ascii_case(&request.owner_pubkey)
            {
                return Err(BrokerError::unauthorized(
                    "the owner key is not a broker requester",
                ));
            }

            let agents = self
                .roster
                .agent_pubkeys(&request.owner_pubkey)
                .await
                .map_err(|error| {
                    // A roster we cannot read is a closed door, not an open one.
                    BrokerError::unauthorized(format!("could not verify agent ownership: {error}"))
                })?;

            let bound = agents
                .iter()
                .any(|pubkey| pubkey.eq_ignore_ascii_case(&request.requester_pubkey));

            if bound {
                Ok(())
            } else {
                Err(BrokerError::unauthorized(
                    "requester is not an agent managed by this owner",
                ))
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use buzz_sdk_pkg::broker::{AgentsCreateArgs, CapabilityArgs};

    use super::*;

    const OWNER: &str = "1111111111111111111111111111111111111111111111111111111111111111";
    const AGENT: &str = "2222222222222222222222222222222222222222222222222222222222222222";
    const STRANGER: &str = "3333333333333333333333333333333333333333333333333333333333333333";
    const CHANNEL: &str = "b2c38ca8-9ec3-411e-bab5-f9deab34d52e";

    struct Roster(Result<Vec<String>, String>);
    impl AgentRoster for Roster {
        fn agent_pubkeys<'a>(
            &'a self,
            _owner_pubkey: &'a str,
        ) -> BoxFuture<'a, Result<Vec<String>, String>> {
            let result = self.0.clone();
            Box::pin(async move { result })
        }
    }

    fn request(requester: &str) -> VerifiedRequest {
        VerifiedRequest {
            owner_pubkey: OWNER.into(),
            requester_pubkey: requester.into(),
            relay_scope: "wss://relay.example".into(),
            payload: Vec::new(),
        }
    }

    fn parsed() -> BrokerRequest {
        BrokerRequest::new(
            "req-1",
            CapabilityArgs::AgentsCreate(AgentsCreateArgs {
                channel_id: CHANNEL.into(),
                display_name: "Helper".into(),
                system_prompt: "Help.".into(),
                runtime: None,
                provider: None,
                model: None,
                respond_to: None,
            }),
        )
        .unwrap()
    }

    async fn authorize(
        roster: Result<Vec<String>, String>,
        requester: &str,
        capability: Capability,
    ) -> Result<(), BrokerError> {
        let authorizer = AgentsAuthorizer::new(Roster(roster));
        let request = request(requester);
        let parsed = parsed();
        authorizer.authorize(&request, capability, &parsed).await
    }

    #[tokio::test]
    async fn an_owners_own_agent_may_manage_agents() {
        for capability in [
            Capability::AgentsCreate,
            Capability::AgentsUpdate,
            Capability::AgentsDelete,
        ] {
            assert!(authorize(Ok(vec![AGENT.into()]), AGENT, capability)
                .await
                .is_ok());
        }
    }

    #[tokio::test]
    async fn a_signer_who_is_not_this_owners_agent_is_refused() {
        let error = authorize(Ok(vec![AGENT.into()]), STRANGER, Capability::AgentsDelete)
            .await
            .unwrap_err();
        assert!(
            error.message.contains("not an agent managed by this owner"),
            "unexpected: {}",
            error.message
        );
    }

    #[tokio::test]
    async fn an_empty_roster_refuses_everyone() {
        assert!(authorize(Ok(Vec::new()), AGENT, Capability::AgentsCreate)
            .await
            .is_err());
    }

    /// The owner key signing its own broker request would erase the delegation
    /// boundary the broker exists to enforce.
    #[tokio::test]
    async fn the_owner_key_is_not_a_requester() {
        let error = authorize(Ok(vec![OWNER.into()]), OWNER, Capability::AgentsCreate)
            .await
            .unwrap_err();
        assert!(
            error
                .message
                .contains("owner key is not a broker requester"),
            "unexpected: {}",
            error.message
        );
    }

    /// A roster that cannot be read must fail closed.
    #[tokio::test]
    async fn an_unreadable_roster_fails_closed() {
        let error = authorize(
            Err("keyring locked".into()),
            AGENT,
            Capability::AgentsUpdate,
        )
        .await
        .unwrap_err();
        assert!(
            error.message.contains("could not verify agent ownership"),
            "unexpected: {}",
            error.message
        );
    }

    #[tokio::test]
    async fn pubkey_comparison_ignores_hex_case() {
        assert!(authorize(
            Ok(vec![AGENT.to_ascii_uppercase()]),
            AGENT,
            Capability::AgentsCreate
        )
        .await
        .is_ok());
    }
}
