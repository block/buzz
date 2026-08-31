//! Reversible whole-community lifecycle commands.

use anyhow::{Context, Result};
use buzz_core::tenant::{normalize_host, relay_url_authority, TenantContext};
use buzz_db::{ArchivedCommunityRecord, UnarchivedCommunityRecord};
use buzz_pubsub::conn_control::ConnControl;
use buzz_pubsub::PubSubManager;
use clap::Subcommand;
use serde_json::{json, Value};
use url::{Host, Url};

const MAX_HOST_LEN: usize = 255;

/// Reversible community lifecycle actions.
#[derive(Debug, Subcommand)]
pub enum CommunitiesCommand {
    /// Archive a community and disconnect its live clients.
    Archive {
        /// Exact community hostname or authority.
        #[arg(long)]
        host: String,
        /// Current owner public key as 64-character hex.
        #[arg(long)]
        owner_pubkey: String,
        /// Identity of the operator performing the action.
        #[arg(long)]
        operator_id: String,
        /// Human-readable reason for the action.
        #[arg(long)]
        reason: String,
    },
    /// Restore an archived community.
    Unarchive {
        /// Exact community hostname or authority.
        #[arg(long)]
        host: String,
        /// Current owner public key as 64-character hex.
        #[arg(long)]
        owner_pubkey: String,
        /// Identity of the operator performing the action.
        #[arg(long)]
        operator_id: String,
        /// Human-readable reason for the action.
        #[arg(long)]
        reason: String,
    },
}

/// Run a reversible community lifecycle action.
pub async fn run(command: CommunitiesCommand) -> anyhow::Result<i32> {
    match command {
        CommunitiesCommand::Archive {
            host,
            owner_pubkey,
            operator_id,
            reason,
        } => archive(host, owner_pubkey, operator_id, reason).await,
        CommunitiesCommand::Unarchive {
            host,
            owner_pubkey,
            operator_id,
            reason,
        } => unarchive(host, owner_pubkey, operator_id, reason).await,
    }
}

async fn archive(
    host: String,
    owner_pubkey: String,
    operator_id: String,
    reason: String,
) -> Result<i32> {
    let host = normalize_host_authority(&host).map_err(anyhow::Error::msg)?;
    let owner_pubkey = parse_owner_pubkey(&owner_pubkey).map_err(anyhow::Error::msg)?;
    let operator_id = required_audit_field("operator_id", &operator_id)
        .map_err(anyhow::Error::msg)?
        .to_string();
    let reason = required_audit_field("reason", &reason)
        .map_err(anyhow::Error::msg)?
        .to_string();

    let relay_url = std::env::var("RELAY_URL")
        .context("RELAY_URL is required to protect the deployment community")?;
    let deployment_host = deployment_host_from_relay_url(&relay_url)?;
    ensure_not_deployment_host(&host, &deployment_host)?;

    let db = crate::connect_db().await?;
    let record = db
        .archive_community_owned_by(&host, &owner_pubkey, &deployment_host)
        .await?
        .ok_or_else(|| {
            anyhow::anyhow!(
                "no active, undeleted community matched both the hostname and current owner pubkey"
            )
        })?;
    let tenant = TenantContext::resolved(record.id, &record.host);

    let command = ConnControl::DisconnectCommunity {
        archived_at: Some(record.archived_at),
    };
    let (propagation, exit_code) = match publish_disconnect(&tenant, &command).await {
        Ok(subscriber_count) => classify_archive_publication(subscriber_count),
        Err(error) => (
            ArchivePropagation::Pending(format!(
                "connection propagation pending — retry this command: {error:#}"
            )),
            1,
        ),
    };
    print_json(&archive_evidence(
        &record,
        &operator_id,
        &reason,
        propagation,
    ))?;
    Ok(exit_code)
}

async fn unarchive(
    host: String,
    owner_pubkey: String,
    operator_id: String,
    reason: String,
) -> Result<i32> {
    let host = normalize_host_authority(&host).map_err(anyhow::Error::msg)?;
    let owner_pubkey = parse_owner_pubkey(&owner_pubkey).map_err(anyhow::Error::msg)?;
    let operator_id = required_audit_field("operator_id", &operator_id)
        .map_err(anyhow::Error::msg)?
        .to_string();
    let reason = required_audit_field("reason", &reason)
        .map_err(anyhow::Error::msg)?
        .to_string();

    let db = crate::connect_db().await?;
    let record = db
        .unarchive_community_owned_by(&host, &owner_pubkey)
        .await?
        .ok_or_else(|| {
            anyhow::anyhow!(
                "no active-deletion-state community matched both the hostname and current owner pubkey"
            )
        })?;
    print_json(&unarchive_evidence(&record, &operator_id, &reason))?;
    Ok(0)
}

async fn publish_disconnect(tenant: &TenantContext, command: &ConnControl) -> Result<i64> {
    let redis_url = std::env::var("REDIS_URL").context("REDIS_URL is required")?;
    let redis_pool = deadpool_redis::Config::from_url(&redis_url)
        .create_pool(Some(deadpool_redis::Runtime::Tokio1))
        .context("Redis pool creation failed")?;
    let pubsub = PubSubManager::new(&redis_url, redis_pool)
        .await
        .context("PubSub init failed")?;
    pubsub
        .publish_conn_control(tenant, command)
        .await
        .context("publishing DisconnectCommunity failed")
}

enum ArchivePropagation {
    Published(i64),
    Pending(String),
}

fn classify_archive_publication(subscriber_count: i64) -> (ArchivePropagation, i32) {
    if subscriber_count > 0 {
        (ArchivePropagation::Published(subscriber_count), 0)
    } else {
        (
            ArchivePropagation::Pending(
                "connection propagation pending — Redis reported zero subscribers; retry this command"
                    .to_string(),
            ),
            1,
        )
    }
}

fn archive_evidence(
    record: &ArchivedCommunityRecord,
    operator_id: &str,
    reason: &str,
    propagation: ArchivePropagation,
) -> Value {
    match propagation {
        ArchivePropagation::Published(subscriber_count) => json!({
            "action": "archive",
            "community_id": record.id.to_string(),
            "host": record.host,
            "archived_at": record.archived_at,
            "status": "archived",
            "operator_id": operator_id,
            "reason": reason,
            "propagation": "published",
            "propagation_subscribers": subscriber_count,
            "retryable": false,
        }),
        ArchivePropagation::Pending(error) => json!({
            "action": "archive",
            "community_id": record.id.to_string(),
            "host": record.host,
            "archived_at": record.archived_at,
            "status": "archived",
            "operator_id": operator_id,
            "reason": reason,
            "propagation": "pending",
            "propagation_subscribers": null,
            "retryable": true,
            "error": error,
        }),
    }
}

fn unarchive_evidence(
    record: &UnarchivedCommunityRecord,
    operator_id: &str,
    reason: &str,
) -> Value {
    json!({
        "action": "unarchive",
        "community_id": record.id.to_string(),
        "host": record.host,
        "archived_at": null,
        "status": "active",
        "operator_id": operator_id,
        "reason": reason,
    })
}

fn print_json(value: &Value) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

fn required_audit_field<'a>(name: &str, value: &'a str) -> Result<&'a str, String> {
    let value = value.trim();
    if value.is_empty() {
        return Err(format!("{name} must not be empty"));
    }
    Ok(value)
}

fn deployment_host_from_relay_url(relay_url: &str) -> Result<String> {
    let host = relay_url_authority(relay_url);
    if host.is_empty() {
        anyhow::bail!("RELAY_URL does not contain a valid deployment authority");
    }
    Ok(host)
}

fn ensure_not_deployment_host(host: &str, deployment_host: &str) -> Result<()> {
    if host == deployment_host {
        anyhow::bail!("the deployment community cannot be archived");
    }
    Ok(())
}

fn normalize_host_authority(host: &str) -> Result<String, String> {
    if host.is_empty() {
        return Err("host is empty".to_string());
    }
    if host.len() > MAX_HOST_LEN {
        return Err(format!(
            "host too long: {} bytes (max {MAX_HOST_LEN})",
            host.len()
        ));
    }
    if host.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return Err("host contains invalid characters".to_string());
    }
    if host.contains('/') || host.contains('?') || host.contains('#') || host.contains('@') {
        return Err(
            "host must be a bare authority (no scheme, path, query, or userinfo)".to_string(),
        );
    }

    let normalized = normalize_host(host);
    let parsed = Url::parse(&format!("http://{normalized}/"))
        .map_err(|_| "host is not a valid authority".to_string())?;
    let parsed_host = parsed
        .host()
        .ok_or_else(|| "host is not a valid authority".to_string())?;
    let serialized_host = match parsed_host {
        Host::Domain(domain) => {
            validate_domain_labels(domain)?;
            domain.to_string()
        }
        Host::Ipv4(addr) => addr.to_string(),
        Host::Ipv6(addr) => format!("[{addr}]"),
    };
    let canonical_authority = match parsed.port() {
        Some(port) => format!("{serialized_host}:{port}"),
        None => serialized_host,
    };
    if canonical_authority != normalized {
        return Err(format!(
            "host is not a canonical authority: expected {canonical_authority:?}"
        ));
    }

    Ok(normalized)
}

fn validate_domain_labels(domain: &str) -> Result<(), String> {
    if domain.len() > 253 {
        return Err("domain name too long".to_string());
    }
    for label in domain.split('.') {
        if label.is_empty() {
            return Err("domain contains an empty label".to_string());
        }
        if label.len() > 63 {
            return Err("domain label too long".to_string());
        }
        let valid_label = label
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
            && !label.starts_with('-')
            && !label.ends_with('-');
        if !valid_label {
            return Err("domain label contains invalid characters".to_string());
        }
    }
    Ok(())
}

fn parse_owner_pubkey(input: &str) -> Result<String, String> {
    let normalized = input.to_ascii_lowercase();
    if normalized.len() != 64 || !normalized.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err("owner pubkey must be 64-character hex".to_string());
    }
    nostr::PublicKey::parse(&normalized)
        .map(|pubkey| pubkey.to_hex())
        .map_err(|error| format!("invalid owner pubkey: {error}"))
}

#[cfg(test)]
mod tests {
    use nostr::Keys;

    use super::*;

    #[test]
    fn normalizes_bare_host_authority() {
        assert_eq!(
            normalize_host_authority("EXAMPLE.COMMUNITIES.BUZZ.XYZ.:443").unwrap(),
            "example.communities.buzz.xyz"
        );
        assert_eq!(
            normalize_host_authority("Relay.Example:8443").unwrap(),
            "relay.example:8443"
        );
    }

    #[test]
    fn rejects_non_authority_and_malformed_hosts() {
        for invalid in [
            "https://relay.example",
            "relay.example/path",
            " relay.example",
            "relay example",
            "relay..example",
            "-relay.example",
            "relay.example:abc",
            "[::1",
        ] {
            assert!(
                normalize_host_authority(invalid).is_err(),
                "accepted invalid host {invalid:?}"
            );
        }
    }

    #[test]
    fn validates_and_canonicalizes_owner_pubkey() {
        let owner = Keys::generate().public_key().to_hex().to_ascii_uppercase();
        assert_eq!(
            parse_owner_pubkey(&owner).unwrap(),
            owner.to_ascii_lowercase()
        );

        let invalid = [
            String::new(),
            "abc".to_string(),
            "g".repeat(64),
            "0".repeat(63),
        ];
        for invalid in &invalid {
            assert!(
                parse_owner_pubkey(invalid).is_err(),
                "accepted invalid owner pubkey {invalid:?}"
            );
        }
    }

    fn archived_record() -> buzz_db::ArchivedCommunityRecord {
        buzz_db::ArchivedCommunityRecord {
            id: buzz_core::CommunityId::from_uuid(uuid::Uuid::from_u128(1)),
            host: "relay.example".to_string(),
            archived_at: "2026-08-31T12:00:00Z".parse().unwrap(),
        }
    }

    #[test]
    fn archive_evidence_records_operator_reason_and_published_propagation() {
        let evidence = archive_evidence(
            &archived_record(),
            "operator@example",
            "owner requested archive",
            ArchivePropagation::Published(3),
        );

        assert_eq!(
            evidence["community_id"],
            uuid::Uuid::from_u128(1).to_string()
        );
        assert_eq!(evidence["host"], "relay.example");
        assert_eq!(evidence["status"], "archived");
        assert_eq!(evidence["operator_id"], "operator@example");
        assert_eq!(evidence["reason"], "owner requested archive");
        assert_eq!(evidence["propagation"], "published");
        assert_eq!(evidence["propagation_subscribers"], 3);
        assert_eq!(evidence["retryable"], false);
    }

    #[test]
    fn archive_evidence_marks_committed_propagation_failure_retryable() {
        let evidence = archive_evidence(
            &archived_record(),
            "operator@example",
            "owner requested archive",
            ArchivePropagation::Pending("redis unavailable".to_string()),
        );

        assert_eq!(evidence["status"], "archived");
        assert_eq!(evidence["propagation"], "pending");
        assert_eq!(evidence["retryable"], true);
        assert_eq!(evidence["error"], "redis unavailable");
    }

    #[test]
    fn zero_redis_subscribers_is_propagation_pending() {
        let (propagation, exit_code) = classify_archive_publication(0);
        let evidence = archive_evidence(
            &archived_record(),
            "operator@example",
            "owner requested archive",
            propagation,
        );

        assert_eq!(exit_code, 1);
        assert_eq!(evidence["propagation"], "pending");
        assert_eq!(evidence["retryable"], true);

        let (propagation, exit_code) = classify_archive_publication(2);
        let evidence = archive_evidence(
            &archived_record(),
            "operator@example",
            "owner requested archive",
            propagation,
        );
        assert_eq!(exit_code, 0);
        assert_eq!(evidence["propagation"], "published");
        assert_eq!(evidence["propagation_subscribers"], 2);
    }

    #[test]
    fn unarchive_evidence_records_operator_reason_without_disconnect() {
        let record = buzz_db::UnarchivedCommunityRecord {
            id: buzz_core::CommunityId::from_uuid(uuid::Uuid::from_u128(2)),
            host: "relay.example".to_string(),
        };
        let evidence = unarchive_evidence(&record, "operator@example", "rollback");

        assert_eq!(
            evidence["community_id"],
            uuid::Uuid::from_u128(2).to_string()
        );
        assert_eq!(evidence["host"], "relay.example");
        assert_eq!(evidence["archived_at"], serde_json::Value::Null);
        assert_eq!(evidence["status"], "active");
        assert_eq!(evidence["operator_id"], "operator@example");
        assert_eq!(evidence["reason"], "rollback");
        assert!(evidence.get("propagation").is_none());
    }

    #[test]
    fn audit_fields_must_be_nonempty() {
        assert_eq!(
            required_audit_field("operator_id", "operator@example").unwrap(),
            "operator@example"
        );
        assert!(required_audit_field("operator_id", " ").is_err());
        assert!(required_audit_field("reason", "\t").is_err());
    }

    #[test]
    fn deployment_host_is_derived_and_protected() {
        assert_eq!(
            deployment_host_from_relay_url("wss://RELAY.EXAMPLE:443/path").unwrap(),
            "relay.example"
        );
        assert!(deployment_host_from_relay_url("not a URL").is_err());
        assert!(ensure_not_deployment_host("relay.example", "relay.example").is_err());
        assert!(ensure_not_deployment_host("other.example", "relay.example").is_ok());
    }
}
