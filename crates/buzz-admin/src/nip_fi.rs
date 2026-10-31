//! NIP-FI community issuer bindings: backfill and explicit operator edits.
//!
//! `backfill` and `bind` read `BUZZ_NIP_FI_PROVISION_BINDING`,
//! `BUZZ_NIP_FI_ISSUERS` and `BUZZ_NIP_FI_COMMUNITIES` with the relay's own
//! parser and overlap check, so the CLI binds exactly what relay provisioning
//! would. `unbind` works without provisioning config. See
//! `docs/operator-nip-fi-bindings.md`.

use anyhow::{Context, Result};
use buzz_auth::ProvisionBindingPolicy;
use buzz_db::{Db, NipFiUnbind};
use clap::Subcommand;
use serde::Serialize;
use serde_json::{json, Value};

use crate::communities::normalize_host_authority;

/// Communities read and bound per backfill page.
const BACKFILL_PAGE: i64 = 1_000;
/// Most unbound hosts listed in the backfill report.
const MAX_REPORTED_HOSTS: usize = 100;

/// NIP-FI binding actions.
#[derive(Debug, Subcommand)]
pub enum NipFiCommand {
    /// Bind every unbound one-label suffix community to the configured
    /// issuers, then report how many remain unbound (must be 0 before enforce).
    Backfill,
    /// Bind one issuer to one community.
    Bind {
        /// Exact community hostname.
        #[arg(long)]
        host: String,
        /// Issuer configured in `BUZZ_NIP_FI_ISSUERS`.
        #[arg(long)]
        issuer: String,
    },
    /// Remove one issuer from one community. Refuses the last issuer.
    Unbind {
        /// Exact community hostname.
        #[arg(long)]
        host: String,
        /// Bound issuer to remove.
        #[arg(long)]
        issuer: String,
    },
}

/// Run a NIP-FI binding action.
pub async fn run(command: NipFiCommand) -> Result<i32> {
    let policy = policy_for(&command, ProvisionBindingPolicy::from_env)?;
    let db = crate::connect_db().await?;
    let (output, code) = match (command, policy) {
        (NipFiCommand::Backfill, Some(policy)) => {
            let report = backfill(&db, &policy).await?;
            (json!(report), report.exit_code())
        }
        (NipFiCommand::Bind { host, issuer }, Some(policy)) => {
            (bind(&db, &policy, &host, &issuer).await?, 0)
        }
        (NipFiCommand::Unbind { host, issuer }, policy) => {
            (unbind(&db, policy.as_ref(), &host, &issuer).await?, 0)
        }
        (_, None) => unreachable!("policy_for requires a policy for backfill and bind"),
    };
    println!("{output}");
    Ok(code)
}

/// The provisioning policy `command` needs: required for `backfill` and
/// `bind`, optional for `unbind`, which runs without it.
fn policy_for(
    command: &NipFiCommand,
    load: impl FnOnce() -> Result<Option<ProvisionBindingPolicy>, buzz_auth::ProvisionBindingError>,
) -> Result<Option<ProvisionBindingPolicy>> {
    if matches!(command, NipFiCommand::Unbind { .. }) {
        return Ok(load().ok().flatten());
    }
    let policy = load()?.context("BUZZ_NIP_FI_PROVISION_BINDING is required")?;
    Ok(Some(policy))
}

/// What a backfill bound and what it left unbound.
#[derive(Debug, PartialEq, Eq, Serialize)]
pub(crate) struct BackfillReport {
    rows_bound: u64,
    unbound: usize,
    unbound_hosts: Vec<String>,
    truncated: bool,
}

impl BackfillReport {
    fn new(rows_bound: u64) -> Self {
        Self {
            rows_bound,
            unbound: 0,
            unbound_hosts: Vec::new(),
            truncated: false,
        }
    }

    /// Count every unbound host, listing at most `MAX_REPORTED_HOSTS`.
    fn add_unbound(&mut self, host: String) {
        self.unbound += 1;
        if self.unbound_hosts.len() < MAX_REPORTED_HOSTS {
            self.unbound_hosts.push(host);
        } else {
            self.truncated = true;
        }
    }

    /// 0 when every bindable community holds a binding, otherwise 1.
    pub(crate) fn exit_code(&self) -> i32 {
        i32::from(self.unbound > 0)
    }
}

/// Bind every unbound, active, bindable community under the policy's suffix,
/// one page of hosts at a time. A community whose deletion starts mid-run is
/// skipped, not an error; the closing recount reports anything left unbound.
pub(crate) async fn backfill(db: &Db, policy: &ProvisionBindingPolicy) -> Result<BackfillReport> {
    let mut rows_bound = 0;
    let mut after = String::new();
    loop {
        let page = db
            .list_unbound_nip_fi_communities(policy.host_suffix(), &after, BACKFILL_PAGE)
            .await?;
        let Some(last) = page.last() else { break };
        after.clone_from(&last.host);
        let bindable: Vec<_> = page
            .iter()
            .filter(|community| policy.is_bindable(&community.host))
            .map(|community| community.id)
            .collect();
        rows_bound += db
            .bind_nip_fi_issuers_to_active(&bindable, policy.authorized_issuers())
            .await?;
    }

    let mut report = BackfillReport::new(rows_bound);
    let mut after = String::new();
    loop {
        let page = db
            .list_unbound_nip_fi_communities(policy.host_suffix(), &after, BACKFILL_PAGE)
            .await?;
        let Some(last) = page.last() else { break };
        after.clone_from(&last.host);
        for community in page {
            if policy.is_bindable(&community.host) {
                report.add_unbound(community.host);
            }
        }
    }
    Ok(report)
}

async fn bind(db: &Db, policy: &ProvisionBindingPolicy, host: &str, issuer: &str) -> Result<Value> {
    let host = normalize_host_authority(host).map_err(anyhow::Error::msg)?;
    policy.check_bind(&host, issuer)?;
    let community = db
        .lookup_community_by_host_for_management(&host)
        .await?
        .context("no community has this host")?;
    let inserted = db
        .bind_nip_fi_issuers(community.id, &[issuer.to_string()])
        .await?;
    Ok(json!({"host": community.host, "issuer": issuer, "bound": inserted > 0}))
}

/// Remove one issuer. The last issuer of a host backfill would rebind is
/// kept; only a policy that proves the host unbindable (a reserved host, or
/// one outside the suffix) lets the last one go.
async fn unbind(
    db: &Db,
    policy: Option<&ProvisionBindingPolicy>,
    host: &str,
    issuer: &str,
) -> Result<Value> {
    let host = normalize_host_authority(host).map_err(anyhow::Error::msg)?;
    let community = db
        .lookup_community_by_host_for_management(&host)
        .await?
        .context("no community has this host")?;
    let allow_last = policy.is_some_and(|policy| !policy.is_bindable(&community.host));
    let removed = match db
        .unbind_nip_fi_issuer(community.id, issuer, allow_last)
        .await?
    {
        NipFiUnbind::Removed => true,
        NipFiUnbind::NotBound => false,
        NipFiUnbind::LastIssuer => anyhow::bail!(
            "refusing to remove the community's last NIP-FI issuer: backfill would rebind it; \
             bind a replacement first, or archive or delete the community to stop it logging in"
        ),
    };
    Ok(json!({"host": community.host, "issuer": issuer, "removed": removed}))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unloadable() -> Result<Option<ProvisionBindingPolicy>, buzz_auth::ProvisionBindingError> {
        ProvisionBindingPolicy::parse("not json", "").map(Some)
    }

    #[test]
    fn unbind_runs_when_the_policy_is_missing_or_fails_to_load() {
        let unbind = NipFiCommand::Unbind {
            host: "acme.example".into(),
            issuer: "https://issuer.example".into(),
        };
        assert!(policy_for(&unbind, unloadable)
            .expect("unbind needs no policy")
            .is_none());
        assert!(policy_for(&unbind, || Ok(None))
            .expect("unbind needs no policy")
            .is_none());
        let bind = NipFiCommand::Bind {
            host: "acme.example".into(),
            issuer: "https://issuer.example".into(),
        };
        for command in [NipFiCommand::Backfill, bind] {
            assert!(policy_for(&command, unloadable).is_err(), "{command:?}");
            assert!(policy_for(&command, || Ok(None)).is_err(), "{command:?}");
        }
    }

    // Exit 1 whenever a bindable host stays unbound; the host list stops at
    // `MAX_REPORTED_HOSTS` and says so; stdout is the report's JSON.
    fn report(rows_bound: u64, hosts: impl IntoIterator<Item = String>) -> BackfillReport {
        let mut report = BackfillReport::new(rows_bound);
        hosts.into_iter().for_each(|host| report.add_unbound(host));
        report
    }

    #[test]
    fn backfill_report_exits_one_caps_its_host_list_and_prints_json() {
        let clean = report(3, Vec::new());
        assert_eq!(clean.exit_code(), 0);
        assert_eq!(
            json!(clean).to_string(),
            r#"{"rows_bound":3,"truncated":false,"unbound":0,"unbound_hosts":[]}"#
        );

        let some = report(0, ["a.example".to_owned()]);
        assert_eq!(some.exit_code(), 1);
        assert_eq!(
            json!(some).to_string(),
            r#"{"rows_bound":0,"truncated":false,"unbound":1,"unbound_hosts":["a.example"]}"#
        );

        let hosts = (0..=MAX_REPORTED_HOSTS).map(|i| format!("h{i}.example"));
        let many = report(0, hosts);
        assert_eq!(many.exit_code(), 1);
        assert_eq!(many.unbound, MAX_REPORTED_HOSTS + 1);
        assert_eq!(many.unbound_hosts.len(), MAX_REPORTED_HOSTS);
        assert_eq!(
            many.unbound_hosts.last().unwrap(),
            &format!("h{}.example", MAX_REPORTED_HOSTS - 1)
        );
        assert!(many.truncated);
        let exact = report(0, (0..MAX_REPORTED_HOSTS).map(|i| format!("h{i}")));
        assert!(!exact.truncated, "a full list is not truncated");
    }
}

#[cfg(test)]
mod postgres_tests {
    use super::*;
    use buzz_db::DbConfig;
    use uuid::Uuid;

    const ISS: &str = "https://issuer.example/api/goose";

    async fn db() -> Db {
        let database_url = std::env::var("BUZZ_TEST_DATABASE_URL")
            .unwrap_or_else(|_| DbConfig::default().database_url);
        Db::new(&DbConfig {
            database_url,
            ..DbConfig::default()
        })
        .await
        .expect("connect to test DB")
    }

    /// A policy under a fresh suffix whose bare suffix host is reserved.
    fn policy(suffix: &str) -> ProvisionBindingPolicy {
        ProvisionBindingPolicy::parse(
            &json!({"host_suffix": suffix, "authorized_issuers": [ISS]}).to_string(),
            &json!([{"issuer": ISS, "command_audiences": [format!("https://cmd.{suffix}")]}])
                .to_string(),
        )
        .expect("valid policy")
    }

    /// Move a community into `quiescing` the way the deletion executor does:
    /// the lifecycle trigger accepts the change only under its GUCs.
    async fn quiesce(id: buzz_core::CommunityId) {
        let url = std::env::var("BUZZ_TEST_DATABASE_URL")
            .unwrap_or_else(|_| DbConfig::default().database_url);
        let pool = sqlx::PgPool::connect(&url).await.expect("connect");
        let mut tx = pool.begin().await.expect("begin");
        sqlx::query(
            "SELECT set_config('buzz.deletion_executor_community', $1, true), \
                    set_config('buzz.deletion_fence_generation', \
                        (SELECT deletion_fence_generation::text FROM communities WHERE id = $2), true)",
        )
        .bind(id.to_string())
        .bind(id.as_uuid())
        .execute(&mut *tx)
        .await
        .expect("authorize lifecycle change");
        sqlx::query("UPDATE communities SET deletion_state = 'quiescing' WHERE id = $1")
            .bind(id.as_uuid())
            .execute(&mut *tx)
            .await
            .expect("quiesce");
        tx.commit().await.expect("commit");
    }

    async fn issuers(db: &Db, host: &str) -> Vec<String> {
        db.lookup_nip_fi_binding_by_host(host)
            .await
            .expect("lookup")
            .expect("community")
            .issuers
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn backfill_binds_only_bindable_hosts_and_reports_the_rest() {
        let db = db().await;
        let suffix = format!("{}.backfill.example", Uuid::new_v4().simple());
        let policy = policy(&suffix);
        let bindable = format!("acme.{suffix}");
        let reserved = format!("cmd.{suffix}");
        let nested = format!("a.b.{suffix}");
        for host in [&bindable, &reserved, &nested] {
            db.ensure_configured_community(host).await.expect("seed");
        }

        let report = backfill(&db, &policy).await.expect("backfill");
        assert_eq!(
            report,
            BackfillReport {
                rows_bound: 1,
                unbound: 0,
                unbound_hosts: Vec::new(),
                truncated: false,
            }
        );
        assert_eq!(report.exit_code(), 0);
        assert_eq!(issuers(&db, &bindable).await, [ISS]);
        assert!(issuers(&db, &reserved).await.is_empty());
        assert!(issuers(&db, &nested).await.is_empty());

        let again = backfill(&db, &policy).await.expect("rerun");
        assert_eq!(again.rows_bound, 0);
        assert_eq!(again.exit_code(), 0);
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn backfill_skips_a_community_being_deleted() {
        let db = db().await;
        let suffix = format!("{}.deleting.example", Uuid::new_v4().simple());
        let policy = policy(&suffix);
        let deleting = format!("deleting.{suffix}");
        let control = format!("control.{suffix}");
        let deleting_id = db
            .ensure_configured_community(&deleting)
            .await
            .expect("seed")
            .id;
        db.ensure_configured_community(&control)
            .await
            .expect("seed");
        quiesce(deleting_id).await;

        let report = backfill(&db, &policy)
            .await
            .expect("backfill must not abort");
        assert_eq!(report.rows_bound, 1);
        assert_eq!(report.exit_code(), 0);
        assert_eq!(issuers(&db, &control).await, [ISS]);
        assert!(issuers(&db, &deleting).await.is_empty());
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn bind_refuses_hosts_outside_the_suffix_and_unbind_keeps_the_last_issuer() {
        let db = db().await;
        let suffix = format!("{}.bind.example", Uuid::new_v4().simple());
        let policy = policy(&suffix);
        let nested = format!("a.b.{suffix}");
        let host = format!("acme.{suffix}");
        db.ensure_configured_community(&nested).await.expect("seed");
        db.ensure_configured_community(&host).await.expect("seed");

        let refused = bind(&db, &policy, &nested, ISS)
            .await
            .expect_err("outside suffix");
        assert_eq!(
            refused.downcast_ref::<buzz_auth::BindRefusal>(),
            Some(&buzz_auth::BindRefusal::OutsideSuffix)
        );
        assert!(issuers(&db, &nested).await.is_empty());

        bind(&db, &policy, &host, ISS).await.expect("bind");
        for policy in [None, Some(&policy)] {
            let last = unbind(&db, policy, &host, ISS)
                .await
                .expect_err("last issuer");
            assert!(last.to_string().contains("archive or delete"), "{last}");
            assert_eq!(issuers(&db, &host).await, [ISS]);
        }

        // A reserved host's binding must be removable, since startup refuses it.
        let reserved = format!("cmd.{suffix}");
        db.ensure_configured_community_with_nip_fi_issuers(&reserved, &[ISS.to_string()])
            .await
            .expect("seed raw reserved binding");
        unbind(&db, Some(&policy), &reserved, ISS)
            .await
            .expect("unbind a reserved host's last issuer");
        assert!(issuers(&db, &reserved).await.is_empty());
    }
}
