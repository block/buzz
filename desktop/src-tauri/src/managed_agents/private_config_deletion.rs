//! Deletion fences survive an absent private patch. Public identity recovery
//! cannot turn the disk migration seed back into configuration authority.
use super::PrivateConfigOverlay;
use crate::managed_agents::retention::{self, get_retained_event, get_retained_events_by_kind};
use buzz_core_pkg::kind::{KIND_DELETION, KIND_MANAGED_AGENT, KIND_PRIVATE_MANAGED_AGENT};

impl PrivateConfigOverlay {
    pub(crate) fn require_config_authority(&self, pubkey: &str) -> Result<(), String> {
        if self.1.contains(pubkey) {
            return Err("private agent configuration was deleted; sync a newer private head from the owning device or delete this identity and create a new agent".into());
        }
        Ok(())
    }

    pub(crate) fn deny_deleted_config(&mut self, pubkey: &str) {
        self.0.remove(pubkey);
        self.1.insert(pubkey.to_string());
    }

    pub(super) fn load_deletion_fences(
        &mut self,
        conn: &rusqlite::Connection,
        keys: &nostr::Keys,
    ) -> Result<(), String> {
        for row in get_retained_events_by_kind(conn, KIND_DELETION, &keys.public_key().to_hex())? {
            if let Some((kind, agent)) = row.d_tag.split_once(':') {
                if matches!(kind, "30177" | "30179") {
                    self.deny_deleted_config(agent);
                }
            }
        }
        Ok(())
    }

    /// Refresh after a retained tombstone even when its newer public sibling
    /// made identity cleanup a no-op. A covered cached private patch still loses.
    pub(crate) fn refresh_config_authority(
        &mut self,
        conn: &rusqlite::Connection,
        keys: &nostr::Keys,
        agent: &str,
    ) -> Result<(), String> {
        let owner = keys.public_key().to_hex();
        if let Some(row) = get_retained_event(conn, KIND_PRIVATE_MANAGED_AGENT, &owner, agent)? {
            if !retention::managed_agent_head_is_deleted(conn, &row)? {
                self.insert_patch(super::patch_from_retained_row(&row, keys)?);
                return Ok(());
            }
            self.deny_deleted_config(agent);
            return Ok(());
        }
        for kind in [KIND_MANAGED_AGENT, KIND_PRIVATE_MANAGED_AGENT] {
            if get_retained_event(
                conn,
                KIND_DELETION,
                &owner,
                &retention::tombstone_retention_d_tag(kind, agent),
            )?
            .is_some()
            {
                self.deny_deleted_config(agent);
                break;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::managed_agents::private_config_overlay::{
        hydrate_from_retention, test_relay_payload,
    };
    use nostr::{JsonUtil, ToBech32};

    #[test]
    fn retained_deleted_private_head_cannot_reenter_through_hydration_or_absorption() {
        let keys = nostr::Keys::generate();
        let agent = nostr::Keys::generate();
        let owner = keys.public_key().to_hex();
        let pubkey = agent.public_key().to_hex();
        let dir = tempfile::tempdir().unwrap();
        let conn = retention::open_retention_db(&dir.path().join("retention.db")).unwrap();
        let mut payload = test_relay_payload(&pubkey);
        payload.owner_pubkey = owner.clone();
        payload.generation = 1;
        payload.identity.private_key_nsec = agent.secret_key().to_bech32().unwrap();
        let event = buzz_core_pkg::private_managed_agent::build_event(&keys, &payload, 10).unwrap();
        retention::retain_event(
            &conn,
            &retention::RetainedEvent {
                kind: KIND_PRIVATE_MANAGED_AGENT,
                pubkey: owner.clone(),
                d_tag: pubkey.clone(),
                content: event.content.clone(),
                created_at: 10,
                raw_event: event.as_json(),
                pending_sync: false,
            },
        )
        .unwrap();
        // Simulate retention from a previous client: the watermark exists but
        // its covered private sibling was not purged. Neither reader may trust it.
        retention::retain_event(
            &conn,
            &retention::RetainedEvent {
                kind: KIND_DELETION,
                pubkey: owner.clone(),
                d_tag: retention::tombstone_retention_d_tag(KIND_MANAGED_AGENT, &pubkey),
                content: String::new(),
                created_at: 20,
                raw_event: "{}".into(),
                pending_sync: false,
            },
        )
        .unwrap();
        let mut overlay = hydrate_from_retention(&conn, &keys).unwrap();
        assert!(overlay.require_config_authority(&pubkey).is_err());
        assert!(overlay
            .materialize_relay_only_record(&pubkey, &[])
            .is_none());
        overlay.absorb_retained_head(&conn, &keys, &pubkey).unwrap();
        assert!(overlay.require_config_authority(&pubkey).is_err());
        assert!(overlay
            .materialize_relay_only_record(&pubkey, &[])
            .is_none());
        overlay.clear();
        assert!(
            overlay.require_config_authority(&pubkey).is_ok(),
            "scope reset clears deletion fences too"
        );
    }
}
