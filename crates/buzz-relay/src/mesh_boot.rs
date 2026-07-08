//! Relay startup wiring for the inter-relay mesh (`BUZZ_MESH` seam).
//!
//! [`boot_mesh`] is the ONLY place the relay constructs mesh machinery. It
//! returns `None` — and touches nothing — when `BUZZ_MESH=off`, so mesh-off
//! deployments stay byte-identical to a relay built before this module
//! existed. When enabled, it:
//!
//! 1. binds the iroh endpoint on `BUZZ_MESH_BIND_ADDR` (boot-unique keypair =
//!    boot-unique `RuntimeId`),
//! 2. publishes a relay-key-attested [`ReadyRecord`] to the Redis ready
//!    registry and starts the readiness-gated heartbeat,
//! 3. starts the [`MeshRuntime`] loops (accept, reconcile/dial, gossip) and
//!    runs one immediate reconcile pass so seed peers are dialed at boot,
//! 4. spawns a drain watcher: when the relay's `shutting_down` flag flips,
//!    membership gossips `draining=true` and the heartbeat clears the
//!    registry record.
//!
//! Consumers (huddle control plane, reliable-stream tunnels) reach the mesh
//! exclusively through [`MeshHandle`] via `AppState::mesh()` — `None` means
//! "behave exactly like a single-instance relay."

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use buzz_relay_mesh::endpoint::MeshEndpoint;
use buzz_relay_mesh::gossip::GossipRecord;
use buzz_relay_mesh::registry::{ReadyRecord, ReadyRegistry};
use buzz_relay_mesh::{
    MeshMembership, MeshRuntime, MeshStatus, RelayMeshMembership, RelayPeerTransport, RuntimeId,
};

use crate::config::Config;
use crate::tunnel::directory::SessionDirectory;

/// Everything a mesh consumer needs, as one bundle.
#[derive(Clone)]
pub struct MeshHandle {
    /// Redis fenced session directory — the ownership arbiter.
    pub directory: SessionDirectory,
    /// Fenced byte transport to peer runtimes.
    pub transport: Arc<dyn RelayPeerTransport>,
    /// Routing hints: who is alive / draining / dialable.
    pub membership: Arc<dyn RelayMeshMembership>,
    /// This runtime's boot-unique mesh identity.
    pub local_runtime_id: RuntimeId,
    /// The running mesh (status snapshots, shutdown).
    runtime: MeshRuntime,
}

impl MeshHandle {
    /// Live `/_mesh` status snapshot.
    pub fn status(&self) -> MeshStatus {
        self.runtime.membership().status()
    }
}

/// Wire protocol version advertised in registry/gossip records.
const PROTO_VERSION: u16 = buzz_relay_mesh::WIRE_VERSION as u16;

/// Capabilities advertised by this build. All three tunnel profiles ship in
/// the same binary, so the list is static.
fn capabilities() -> Vec<String> {
    vec![
        "reliable-stream".to_string(),
        "realtime-media".to_string(),
        "huddle-control".to_string(),
    ]
}

/// Addresses peers should dial, in preference order:
/// `BUZZ_MESH_ADVERTISE_ADDR` (explicit, classic-LB shapes) →
/// `POD_IP` + actual bound port (k8s Downward API, zero RBAC) →
/// every IP transport addr the endpoint reports (dev/local).
fn advertise_addrs(endpoint: &MeshEndpoint) -> Vec<String> {
    if let Ok(addr) = std::env::var("BUZZ_MESH_ADVERTISE_ADDR") {
        let addr = addr.trim().to_string();
        if !addr.is_empty() {
            return vec![addr];
        }
    }

    let ip_addrs = endpoint.ip_addrs();
    let bound_port = ip_addrs.first().map(|sock| sock.port()).unwrap_or(0);

    if let Ok(pod_ip) = std::env::var("POD_IP") {
        let pod_ip = pod_ip.trim();
        if !pod_ip.is_empty() && bound_port != 0 {
            return vec![format!("{pod_ip}:{bound_port}")];
        }
    }

    ip_addrs.iter().map(|sock| sock.to_string()).collect()
}

/// Boot the mesh, or return `None` when `BUZZ_MESH=off`.
///
/// Never fatal to relay startup by policy? No — a *misconfigured* enabled mesh
/// fails loudly (bind failure, Redis unreachable at publish). An operator who
/// sets `BUZZ_MESH=on` wants the mesh or wants to know why not; silently
/// booting meshless would be the same class of bug as silently dropping to a
/// default tenant.
pub async fn boot_mesh(
    config: &Config,
    redis_pool: deadpool_redis::Pool,
    relay_keypair: &nostr::Keys,
    shutting_down: Arc<AtomicBool>,
) -> anyhow::Result<Option<MeshHandle>> {
    if !config.mesh.enabled {
        tracing::info!("mesh disabled (BUZZ_MESH=off) — single-instance behavior");
        return Ok(None);
    }

    let endpoint = MeshEndpoint::bind(config.mesh.bind_addr)
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "mesh endpoint bind on {} failed: {e}",
                config.mesh.bind_addr
            )
        })?;
    let runtime_id = endpoint.runtime_id();
    let addrs = advertise_addrs(&endpoint);
    tracing::info!(
        runtime_id = %runtime_id,
        bind_addr = %config.mesh.bind_addr,
        advertise_addrs = ?addrs,
        "mesh endpoint bound"
    );

    let mut local_record = GossipRecord::new(runtime_id, addrs.clone(), PROTO_VERSION);
    local_record.capabilities = capabilities();
    let membership = MeshMembership::new(local_record);

    let registry = ReadyRegistry::new(redis_pool.clone(), config.mesh.registry_refresh);
    let ready_record = ReadyRecord::new(
        runtime_id,
        relay_keypair,
        addrs,
        PROTO_VERSION,
        capabilities(),
    );

    // First publish is part of boot: if Redis can't take the attested record,
    // peers can never find us — fail loudly now, not quietly forever.
    registry
        .publish_ready(&ready_record)
        .await
        .map_err(|e| anyhow::anyhow!("mesh ready-registry publish failed: {e}"))?;
    tracing::info!(runtime_id = %runtime_id, "mesh ready record published");

    // Readiness-gated heartbeat: publishes while the relay would pass
    // readiness, clears the record on ready→not-ready and on shutdown.
    let hb_flag = Arc::clone(&shutting_down);
    buzz_relay_mesh::runtime::spawn_registry_heartbeat(
        registry.clone(),
        ready_record,
        Arc::new(move || !hb_flag.load(Ordering::Relaxed)),
    );

    let runtime = MeshRuntime::start(endpoint, membership, Some(registry));
    // Dial seed peers now rather than waiting for the first reconcile tick.
    runtime.reconcile_now().await;

    // Drain watcher: SIGTERM flips `shutting_down`; gossip `draining=true` so
    // peers stop routing new sessions here while in-flight ones drain.
    {
        let runtime = runtime.clone();
        let flag = shutting_down;
        tokio::spawn(async move {
            loop {
                if flag.load(Ordering::Relaxed) {
                    runtime.membership().begin_drain();
                    tracing::info!("mesh drain started (draining=true gossiped)");
                    return;
                }
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }
        });
    }

    let membership_arc: Arc<dyn RelayMeshMembership> = Arc::new(runtime.membership().clone());
    let transport: Arc<dyn RelayPeerTransport> = Arc::new(runtime.clone());

    Ok(Some(MeshHandle {
        directory: SessionDirectory::new(redis_pool),
        transport,
        membership: membership_arc,
        local_runtime_id: runtime_id,
        runtime,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// BUZZ_MESH=off must be a hard no-op: no endpoint bind, no Redis write,
    /// no background task — `boot_mesh` returns `None` before touching
    /// anything. The Redis pool here points nowhere routable; if the off path
    /// ever reached Redis this test would hang/fail.
    #[tokio::test]
    async fn mesh_off_boots_nothing() {
        let mut config = crate::config::Config::from_env().expect("default config loads");
        config.mesh.enabled = false;
        let pool = deadpool_redis::Config::from_url("redis://127.0.0.1:1") // unroutable
            .create_pool(Some(deadpool_redis::Runtime::Tokio1))
            .unwrap();
        let keys = nostr::Keys::generate();
        let handle = boot_mesh(&config, pool, &keys, Arc::new(AtomicBool::new(false)))
            .await
            .expect("off path is never an error");
        assert!(handle.is_none());
    }
}
