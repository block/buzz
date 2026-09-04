//! Destination-local keyless launch boundary. Credentials never cross IPC/relay.
//! Automatic issuance is not in the merged broker contract. Keep the production
//! provider fail-closed until that host-owned adapter is supplied; do not mint a
//! competing bearer token or fall back to exporting the agent key.
use super::ManagedAgentRecord;
use buzz_core_pkg::desktop_lifecycle::Outcome;
use std::process::Command;

pub(crate) struct LaunchScope<'a> {
    pub owner: &'a str,
    pub community: &'a str,
    pub agent: &'a str,
}

/// A host-issued session, bound to this destination's independently resolved
/// inputs. No Debug/Serialize: the bearer credential is not diagnostic data.
pub(crate) struct BrokerSession {
    owner: String,
    community: String,
    agent: String,
    endpoint: String,
    credential: zeroize::Zeroizing<String>,
    channels: Vec<String>,
    expires_at: u64,
}

impl BrokerSession {
    /// Future host provisioning adapter calls this with its authenticated reply,
    /// never values from an incoming lifecycle command or user environment.
    #[allow(dead_code)] // consumed by the pending automatic-issuance host adapter
    pub(crate) fn from_host(
        scope: LaunchScope<'_>,
        endpoint: String,
        credential: String,
        channels: Vec<String>,
        expires_at: u64,
    ) -> Result<Self, String> {
        let url = url::Url::parse(&endpoint).map_err(|_| "Invalid broker endpoint")?;
        if !matches!(url.scheme(), "https" | "http")
            || url.host_str().is_none()
            || (url.scheme() == "http"
                && !matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]")))
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || credential.trim().is_empty()
            || channels.is_empty()
            || channels.len() > 256
            || channels.iter().any(|c| uuid::Uuid::parse_str(c).is_err())
        {
            return Err("Invalid broker provisioning".into());
        }
        let session = Self {
            owner: scope.owner.into(),
            community: scope.community.into(),
            agent: scope.agent.into(),
            endpoint,
            credential: zeroize::Zeroizing::new(credential),
            channels,
            expires_at,
        };
        session.validate(scope)?;
        Ok(session)
    }
    pub(crate) fn validate(&self, scope: LaunchScope<'_>) -> Result<(), String> {
        if self.owner != scope.owner
            || self.community != scope.community
            || self.agent != scope.agent
            || self.expires_at <= nostr::Timestamp::now().as_secs().saturating_add(30)
        {
            return Err("Broker session expired or scope changed".into());
        }
        Ok(())
    }
    /// Applied last, after local user/provider configuration. Never let saved
    /// environment turn a keyless launch into keyful authentication.
    pub(crate) fn apply(
        &self,
        command: &mut Command,
        scope: LaunchScope<'_>,
    ) -> Result<(), String> {
        self.validate(scope)?;
        for key in [
            "BUZZ_PRIVATE_KEY",
            "NOSTR_PRIVATE_KEY",
            "BUZZ_AUTH_TAG",
            "BUZZ_API_TOKEN",
            "BUZZ_ACP_PRIVATE_KEY",
            "BUZZ_ACP_API_TOKEN",
            "BUZZ_RELAY_URL",
            "BUZZ_ACP_RELAY_URL",
            "BUZZ_ACP_RESPOND_TO_ALLOWLIST",
            "GIT_CONFIG_COUNT",
        ] {
            command.env_remove(key);
        }
        command
            .env("BUZZ_AGENT_MODE", "broker")
            .env("BUZZ_BROKER_URL", &self.endpoint)
            .env("BUZZ_BROKER_CREDENTIAL", self.credential.as_str())
            .env("BUZZ_BROKER_RELAY_URL", &self.community)
            .env("BUZZ_ACP_AGENT_OWNER", &self.owner)
            .env("BUZZ_ACP_CHANNELS", self.channels.join(","))
            .env("BUZZ_ACP_RESPOND_TO", "owner-only")
            .env("BUZZ_ACP_ALLOWED_RESPOND_TO", "owner-only");
        Ok(())
    }
}

/// Explicit external integration gate, not a fabricated successful launch.
pub(crate) fn provision(
    _scope: LaunchScope<'_>,
    _record: &ManagedAgentRecord,
) -> Result<BrokerSession, Outcome> {
    Err(Outcome::ProvisioningUnavailable)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn scope() -> LaunchScope<'static> {
        LaunchScope {
            owner: "owner",
            community: "wss://one.example",
            agent: "agent",
        }
    }
    #[test]
    fn scope_expiry_and_final_environment_are_enforced() {
        let session = BrokerSession::from_host(
            scope(),
            "https://broker.example".into(),
            "secret".into(),
            vec![uuid::Uuid::new_v4().to_string()],
            nostr::Timestamp::now().as_secs() + 300,
        )
        .unwrap();
        assert!(session
            .validate(LaunchScope {
                community: "wss://other.example",
                ..scope()
            })
            .is_err());
        let mut command = Command::new("unused");
        command
            .env("BUZZ_PRIVATE_KEY", "must-not-escape")
            .env("NOSTR_PRIVATE_KEY", "must-not-escape")
            .env("BUZZ_AGENT_MODE", "local");
        session.apply(&mut command, scope()).unwrap();
        let env: std::collections::BTreeMap<_, _> = command.get_envs().collect();
        assert_eq!(
            env.get(std::ffi::OsStr::new("BUZZ_PRIVATE_KEY")),
            Some(&None)
        );
        assert_eq!(
            env.get(std::ffi::OsStr::new("NOSTR_PRIVATE_KEY")),
            Some(&None)
        );
        assert_eq!(
            env.get(std::ffi::OsStr::new("BUZZ_AGENT_MODE")),
            Some(&Some(std::ffi::OsStr::new("broker")))
        );
        assert!(BrokerSession::from_host(
            scope(),
            "http://remote.example".into(),
            "secret".into(),
            vec![uuid::Uuid::new_v4().to_string()],
            nostr::Timestamp::now().as_secs() + 300
        )
        .is_err());
        assert!(BrokerSession::from_host(
            scope(),
            "https://broker.example".into(),
            "secret".into(),
            vec![uuid::Uuid::new_v4().to_string()],
            0
        )
        .is_err());
    }
}
