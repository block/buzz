//! One presence generation per harness process, independent of parallel sessions.
use crate::relay::{RelayError, RelayEventPublisher};
use buzz_core::run_presence::{self, Location};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};

#[derive(Clone)]
pub(crate) struct PresencePublisher {
    run: String,
    seq: Arc<AtomicU64>,
    location: Option<Location>,
}
impl PresencePublisher {
    pub(crate) fn from_env() -> Result<Self, String> {
        let location = std::env::var("BUZZ_ACP_HOST_PUBKEY")
            .ok()
            .zip(std::env::var("BUZZ_ACP_HOST_LABEL").ok())
            .map(|(host, label)| Location { host, label })
            .filter(|l| l.validate().is_ok());
        let launcher = match std::env::var("BUZZ_MANAGED_AGENT_START_NONCE") {
            Ok(value) => Some(value),
            Err(std::env::VarError::NotPresent) => None,
            Err(_) => return Err("invalid launcher run generation".into()),
        };
        Ok(Self {
            run: run_generation(launcher.as_deref())?,
            seq: Arc::new(AtomicU64::new(0)),
            location,
        })
    }
    pub(crate) async fn publish(
        &self,
        publisher: &RelayEventPublisher,
        keys: &nostr::Keys,
        status: &str,
    ) -> Result<(), RelayError> {
        let event = run_presence::pulse(
            keys,
            &self.run,
            self.seq.fetch_add(1, Ordering::SeqCst),
            status,
            self.location.as_ref(),
            None,
            nostr::Timestamp::now().as_secs(),
        )
        .map_err(RelayError::Http)?;
        publisher.publish_event(event).await?;
        Ok(())
    }
}

// The launcher generation is the public run ID, not a second random identity.
// Standalone harnesses still generate their own run. A malformed launcher ID
// must fail startup rather than publish a run the controller cannot fence.
fn run_generation(launcher: Option<&str>) -> Result<String, String> {
    match launcher {
        Some(id)
            if id.len() == 32
                && id
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) =>
        {
            Ok(id.to_owned())
        }
        Some(_) => Err("invalid launcher run generation".into()),
        None => Ok(uuid::Uuid::new_v4().simple().to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn presence_uses_exact_launcher_generation() {
        let id = "ab".repeat(16);
        assert_eq!(run_generation(Some(&id)).unwrap(), id);
        for bad in ["", "test-generation", &"AB".repeat(16), &"aa".repeat(32)] {
            assert!(run_generation(Some(bad)).is_err());
        }
        assert_ne!(run_generation(None).unwrap(), run_generation(None).unwrap());
    }
}
