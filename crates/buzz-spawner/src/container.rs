//! Container lifecycle: the [`ContainerOps`] boundary and its Docker backend.
//!
//! Agents run shell and file-edit tools through `buzz-dev-mcp`, which is
//! arbitrary code execution by design. Running several of them as plain
//! subprocesses of one daemon would let any agent read every other agent's
//! workspace and the spawner's own state file — including every other agent's
//! secret key. So each agent gets its own container, its own volume, and its own
//! resource ceiling.
//!
//! Everything that touches Docker sits behind [`ContainerOps`] so the reconciler
//! can be tested against an in-memory fake.

use std::collections::HashMap;
use std::time::Duration;

use anyhow::{Context, Result};
use async_trait::async_trait;

use crate::config::{AGENT_LABEL, SLUG_LABEL, SPAWNER_LABEL};

/// A container the spawner is responsible for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedContainer {
    /// Docker container id.
    pub id: String,
    /// Container name.
    pub name: String,
    /// Agent pubkey from the `com.buzz.agent` label.
    pub agent_pubkey: String,
    /// Spec slug from the `com.buzz.spec-slug` label.
    pub slug: String,
    /// Whether the container is currently running.
    pub running: bool,
}

/// Everything needed to create one agent container.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContainerSpec {
    /// Container name.
    pub name: String,
    /// Image reference.
    pub image: String,
    /// Agent pubkey, applied as a label for reconciliation.
    pub agent_pubkey: String,
    /// Spec slug, applied as a label.
    pub slug: String,
    /// Spawner pubkey, applied as a label so co-tenant spawners on one host do
    /// not reap each other's containers.
    pub spawner_pubkey: String,
    /// Full environment for the harness process.
    pub env: Vec<(String, String)>,
    /// CPU allocation in thousandths of a core.
    pub cpu_millis: u32,
    /// Memory limit in mebibytes.
    pub memory_mib: u32,
    /// Named volume mounted at the agent's nest directory.
    pub volume_name: String,
}

impl ContainerSpec {
    /// Docker labels identifying this container's owner and purpose.
    pub fn labels(&self) -> HashMap<String, String> {
        HashMap::from([
            (AGENT_LABEL.to_string(), self.agent_pubkey.clone()),
            (SLUG_LABEL.to_string(), self.slug.clone()),
            (SPAWNER_LABEL.to_string(), self.spawner_pubkey.clone()),
        ])
    }
}

/// The container operations the reconciler needs.
#[async_trait]
pub trait ContainerOps: Send + Sync {
    /// List every container labelled as belonging to `spawner_pubkey`,
    /// running or not.
    async fn list(&self, spawner_pubkey: &str) -> Result<Vec<ManagedContainer>>;

    /// Create and start a container.
    async fn create(&self, spec: &ContainerSpec) -> Result<String>;

    /// Stop and remove a container, along with its volume when `purge_volume`
    /// is set. Removing a container the caller believes is gone is not an error.
    async fn remove(&self, container_id: &str, purge_volume: Option<&str>) -> Result<()>;

    /// Tail the last `lines` of a container's logs.
    async fn logs(&self, container_id: &str, lines: usize) -> Result<String>;
}

/// Timeout for a container listing.
///
/// Every Docker call is time-boxed. bollard will wait indefinitely on a
/// half-open socket, and the reconcile loop awaits these calls inline — so one
/// stalled request (a daemon restart, a laptop resuming from sleep) hangs the
/// entire daemon *permanently and silently*: no ticker, no relay reads, no
/// status updates, every agent frozen in whatever phase it was in. A timeout
/// turns that into an error the loop logs and retries on the next pass.
const LIST_TIMEOUT: Duration = Duration::from_secs(30);

/// Timeout for creating a container. Generous because it may pull an image.
const CREATE_TIMEOUT: Duration = Duration::from_secs(600);

/// Timeout for removing a container and optionally its volume.
const REMOVE_TIMEOUT: Duration = Duration::from_secs(120);

/// Timeout for reading container logs.
const LOGS_TIMEOUT: Duration = Duration::from_secs(30);

/// Run a Docker operation under a deadline, naming it in the error.
async fn with_timeout<T>(
    what: &str,
    limit: Duration,
    op: impl std::future::Future<Output = Result<T>>,
) -> Result<T> {
    match tokio::time::timeout(limit, op).await {
        Ok(result) => result,
        Err(_) => anyhow::bail!(
            "docker {what} timed out after {}s; the daemon may be unreachable",
            limit.as_secs()
        ),
    }
}

/// The Docker-socket backend.
pub struct DockerOps {
    docker: bollard::Docker,
}

impl DockerOps {
    /// Connect to the local Docker daemon over its Unix socket (or the platform
    /// default), honoring `DOCKER_HOST` when set.
    pub fn connect() -> Result<Self> {
        let docker = bollard::Docker::connect_with_defaults()
            .context("failed to connect to the Docker daemon")?;
        Ok(Self { docker })
    }
}

#[async_trait]
impl ContainerOps for DockerOps {
    async fn list(&self, spawner_pubkey: &str) -> Result<Vec<ManagedContainer>> {
        with_timeout("list", LIST_TIMEOUT, self.list_inner(spawner_pubkey)).await
    }

    async fn create(&self, spec: &ContainerSpec) -> Result<String> {
        with_timeout("create", CREATE_TIMEOUT, self.create_inner(spec)).await
    }

    async fn remove(&self, container_id: &str, purge_volume: Option<&str>) -> Result<()> {
        with_timeout(
            "remove",
            REMOVE_TIMEOUT,
            self.remove_inner(container_id, purge_volume),
        )
        .await
    }

    async fn logs(&self, container_id: &str, lines: usize) -> Result<String> {
        with_timeout("logs", LOGS_TIMEOUT, self.logs_inner(container_id, lines)).await
    }
}

impl DockerOps {
    async fn list_inner(&self, spawner_pubkey: &str) -> Result<Vec<ManagedContainer>> {
        use bollard::query_parameters::ListContainersOptions;

        let mut filters = HashMap::new();
        filters.insert(
            "label".to_string(),
            vec![format!("{SPAWNER_LABEL}={spawner_pubkey}")],
        );

        let containers = self
            .docker
            .list_containers(Some(ListContainersOptions {
                all: true,
                filters: Some(filters),
                ..Default::default()
            }))
            .await
            .context("failed to list agent containers")?;

        Ok(containers
            .into_iter()
            .filter_map(|c| {
                let labels = c.labels.unwrap_or_default();
                Some(ManagedContainer {
                    id: c.id?,
                    name: c
                        .names
                        .and_then(|n| n.first().cloned())
                        .unwrap_or_default()
                        .trim_start_matches('/')
                        .to_string(),
                    // A container carrying our spawner label but missing the
                    // agent label is malformed; skipping it is safer than
                    // guessing an identity for something we would then act on.
                    agent_pubkey: labels.get(AGENT_LABEL)?.clone(),
                    slug: labels.get(SLUG_LABEL).cloned().unwrap_or_default(),
                    running: c.state.as_ref().is_some_and(|s| {
                        matches!(s, bollard::models::ContainerSummaryStateEnum::RUNNING)
                    }),
                })
            })
            .collect())
    }

    async fn create_inner(&self, spec: &ContainerSpec) -> Result<String> {
        use bollard::models::{ContainerCreateBody, HostConfig, Mount, MountType};
        use bollard::query_parameters::{
            CreateContainerOptions, CreateImageOptionsBuilder, StartContainerOptions,
        };
        use futures_util::StreamExt;

        // Pull so a missing image surfaces as a clear status error rather than a
        // create failure the owner cannot interpret.
        //
        // A pull failure is only fatal when the image is also absent locally. An
        // operator running a locally-built image — an air-gapped host, a
        // pre-loaded tarball, or an image built on the box itself — has nothing
        // to pull from, and failing there would make those setups impossible.
        // Checking local presence second also means a transient registry outage
        // does not stop an already-cached image from starting.
        let mut pull = self.docker.create_image(
            Some(
                CreateImageOptionsBuilder::default()
                    .from_image(&spec.image)
                    .build(),
            ),
            None,
            None,
        );
        let mut pull_error: Option<bollard::errors::Error> = None;
        while let Some(chunk) = pull.next().await {
            if let Err(e) = chunk {
                pull_error = Some(e);
                break;
            }
        }
        if let Some(e) = pull_error {
            if self.docker.inspect_image(&spec.image).await.is_err() {
                return Err(e).with_context(|| {
                    format!(
                        "failed to pull image {} and it is not present locally",
                        spec.image
                    )
                });
            }
            tracing::debug!(
                image = %spec.image,
                "image pull failed but a local copy exists; using it: {e}"
            );
        }

        let host_config = HostConfig {
            // NanoCPUs is billionths of a core; cpu_millis is thousandths.
            nano_cpus: Some(i64::from(spec.cpu_millis) * 1_000_000),
            memory: Some(i64::from(spec.memory_mib) * 1024 * 1024),
            mounts: Some(vec![Mount {
                target: Some("/home/agent/.buzz".to_string()),
                source: Some(spec.volume_name.clone()),
                typ: Some(MountType::VOLUME),
                ..Default::default()
            }]),
            restart_policy: None,
            ..Default::default()
        };

        let body = ContainerCreateBody {
            image: Some(spec.image.clone()),
            env: Some(
                spec.env
                    .iter()
                    .map(|(k, v)| format!("{k}={v}"))
                    .collect::<Vec<_>>(),
            ),
            labels: Some(spec.labels()),
            host_config: Some(host_config),
            ..Default::default()
        };

        let created = match self
            .docker
            .create_container(
                Some(CreateContainerOptions {
                    name: Some(spec.name.clone()),
                    ..Default::default()
                }),
                body.clone(),
            )
            .await
        {
            Ok(created) => created,
            // The name can be squatted by a container a previous spawner
            // The name can be squatted by a container we created and then lost
            // track of (a crash between create and the store write) — self-heal
            // by removing it, but only after the labels confirm it is an agent
            // container belonging to this spawner.
            Err(e) if is_name_conflict(&e) => {
                self.remove_name_squatter(&spec.name, &spec.spawner_pubkey)
                    .await
                    .with_context(|| {
                        format!(
                            "container name {} is taken and could not be reclaimed",
                            spec.name
                        )
                    })?;
                self.docker
                    .create_container(
                        Some(CreateContainerOptions {
                            name: Some(spec.name.clone()),
                            ..Default::default()
                        }),
                        body,
                    )
                    .await
                    .with_context(|| {
                        format!(
                            "failed to create container {} after reclaiming its name",
                            spec.name
                        )
                    })?
            }
            Err(e) => {
                return Err(e).with_context(|| format!("failed to create container {}", spec.name));
            }
        };

        self.docker
            .start_container(&created.id, None::<StartContainerOptions>)
            .await
            .with_context(|| format!("failed to start container {}", spec.name))?;

        Ok(created.id)
    }

    async fn remove_inner(&self, container_id: &str, purge_volume: Option<&str>) -> Result<()> {
        use bollard::query_parameters::{RemoveContainerOptions, RemoveVolumeOptions};

        // force stops a running container; the reconciler's intent is "gone",
        // and a graceful stop is the harness's job via the shutdown convention.
        let removed = self
            .docker
            .remove_container(
                container_id,
                Some(RemoveContainerOptions {
                    force: true,
                    v: true,
                    ..Default::default()
                }),
            )
            .await;

        // A container that is already gone is the desired state, not an error.
        if let Err(e) = removed {
            if !is_not_found(&e) {
                return Err(e)
                    .with_context(|| format!("failed to remove container {container_id}"));
            }
        }

        if let Some(volume) = purge_volume {
            let removed = self
                .docker
                .remove_volume(volume, None::<RemoveVolumeOptions>)
                .await;
            if let Err(e) = removed {
                if !is_not_found(&e) {
                    return Err(e).with_context(|| format!("failed to remove volume {volume}"));
                }
            }
        }

        Ok(())
    }

    /// Remove an agent container squatting on `name`.
    ///
    /// Two gates, both required. The container must carry the `com.buzz.agent`
    /// label, so an operator's unrelated container of the same name is never
    /// touched. And its `com.buzz.spawner` label must name *us* — a container
    /// belonging to another spawner identity is left alone even though the name
    /// blocks us, because that spawner may be alive and still serving it, and
    /// force-removing a live agent to free a name is never the right trade.
    ///
    /// What that leaves is the case this exists for: a leftover from a crash
    /// between `create` and the store write, under our own identity. Names
    /// carry the spawner prefix (see [`AgentRecord::container_name`]), so a
    /// conflict from any other identity means a genuine collision an operator
    /// needs to see rather than something to silently resolve.
    ///
    /// The workspace volume is a named mount and survives the removal.
    async fn remove_name_squatter(&self, name: &str, spawner_pubkey: &str) -> Result<()> {
        use bollard::query_parameters::InspectContainerOptions;

        let squatter = self
            .docker
            .inspect_container(name, None::<InspectContainerOptions>)
            .await
            .with_context(|| format!("failed to inspect conflicting container {name}"))?;
        let labels = squatter.config.as_ref().and_then(|c| c.labels.as_ref());
        if !labels.is_some_and(|labels| labels.contains_key(AGENT_LABEL)) {
            anyhow::bail!(
                "conflicting container {name} does not carry the {AGENT_LABEL} label; \
                 refusing to remove a container the spawner does not manage"
            );
        }
        let owner = labels
            .and_then(|labels| labels.get(SPAWNER_LABEL))
            .map(String::as_str)
            .unwrap_or_default();
        if owner != spawner_pubkey {
            anyhow::bail!(
                "conflicting container {name} belongs to spawner {owner}, not {spawner_pubkey}; \
                 refusing to remove another spawner's agent to free the name"
            );
        }
        tracing::info!(
            container = %name,
            "removing our own orphaned agent container squatting on required name"
        );
        self.remove_inner(name, None).await
    }

    async fn logs_inner(&self, container_id: &str, lines: usize) -> Result<String> {
        use bollard::query_parameters::LogsOptionsBuilder;
        use futures_util::StreamExt;

        let mut stream = self.docker.logs(
            container_id,
            Some(
                LogsOptionsBuilder::default()
                    .stdout(true)
                    .stderr(true)
                    .tail(&lines.to_string())
                    .build(),
            ),
        );

        let mut out = String::new();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.with_context(|| format!("failed to read logs for {container_id}"))?;
            out.push_str(&chunk.to_string());
        }
        Ok(out)
    }
}

/// True when Docker rejected a create because the container name is taken.
fn is_name_conflict(e: &bollard::errors::Error) -> bool {
    matches!(
        e,
        bollard::errors::Error::DockerResponseServerError {
            status_code: 409,
            ..
        }
    )
}

fn is_not_found(e: &bollard::errors::Error) -> bool {
    matches!(
        e,
        bollard::errors::Error::DockerResponseServerError {
            status_code: 404,
            ..
        }
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_carry_agent_slug_and_spawner() {
        let spec = ContainerSpec {
            name: "buzz-agent-abc-fizz".into(),
            image: "img".into(),
            agent_pubkey: "a".repeat(64),
            slug: "fizz".into(),
            spawner_pubkey: "s".repeat(64),
            env: vec![],
            cpu_millis: 1000,
            memory_mib: 2048,
            volume_name: "vol".into(),
        };
        let labels = spec.labels();
        assert_eq!(labels.get(AGENT_LABEL), Some(&"a".repeat(64)));
        assert_eq!(labels.get(SLUG_LABEL), Some(&"fizz".to_string()));
        // Without this, two spawners on one host reap each other's containers.
        assert_eq!(labels.get(SPAWNER_LABEL), Some(&"s".repeat(64)));
    }

    #[test]
    fn name_conflict_matches_409_only() {
        let conflict = bollard::errors::Error::DockerResponseServerError {
            status_code: 409,
            message: "Conflict. The container name is already in use".into(),
        };
        assert!(is_name_conflict(&conflict));
        let not_found = bollard::errors::Error::DockerResponseServerError {
            status_code: 404,
            message: "no such container".into(),
        };
        assert!(!is_name_conflict(&not_found));
    }
}
