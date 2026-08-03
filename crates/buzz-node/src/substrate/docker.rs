//! Docker substrate: runs each workload body as a Docker container.
//!
//! A deploy replaces any previous container for the workload (`docker rm -f`,
//! then `docker run -d`) using an agent body image built from
//! `Dockerfile.agent` — `buzz-acp` as the entrypoint. The image comes in
//! per-runtime variants (the `RUNTIME` build arg): the slim default carries
//! only the sprig personalities (`buzz-agent`, `buzz-dev-mcp`), and each
//! catalog runtime of [`env::known_runtime`] with its own tooling — Goose,
//! the Claude Code and Codex CLIs with their npm ACP adapters — ships as its
//! own variant. The substrate resolves the image per workload runtime (see
//! [`DockerSubstrateConfig::image`]) and fails a deploy closed when the
//! resolved image is not present on the node — it never pulls or builds. The
//! harness environment contract is shared with the process substrate
//! ([`super::env`]); it reaches the container through a short-lived `0600`
//! env-file (plus name-only `-e` pass-through for values an env-file cannot
//! carry), never through command-line arguments.
//!
//! ## The container is the key store
//!
//! Unlike the process substrate, which holds launch keys in process memory,
//! this substrate deliberately keeps **no** key material of its own: the
//! one-time launch key lives in the container's environment, owned by the
//! Docker daemon. `start` and `restart` therefore survive a node restart —
//! they revive the existing container, key and all — and fail closed the
//! moment the container is gone, because nothing else on the node knows the
//! key. A keyless redeploy recovers the key from the existing container
//! (`docker inspect`) before replacing it.
//!
//! ## Exit supervision
//!
//! Every running container gets a `docker wait` watcher that reports bodies
//! exiting on their own (never exits this substrate caused). On construction
//! the substrate re-arms watchers for all running containers carrying its
//! workload labels, so bodies that outlived a node restart stay supervised.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use buzz_core::execution::{SafeErrorCode, WorkloadId, WorkloadSpec};
use tokio::sync::{mpsc, Mutex};
use tracing::warn;
use zeroize::Zeroizing;

use super::{env, Substrate, SubstrateError, WorkloadExit};

/// Container label carrying the owner scope of a workload container.
const LABEL_OWNER: &str = "buzz.node.owner";

/// Container label carrying the workload identity of a workload container.
const LABEL_WORKLOAD: &str = "buzz.node.workload";

/// Hostname containers use to reach services listening on the node host.
const HOST_GATEWAY_NAME: &str = "host.docker.internal";

/// Loopback hosts that are unreachable from inside a container and get
/// rewritten to [`HOST_GATEWAY_NAME`].
const LOOPBACK_HOSTS: &[&str] = &["localhost", "127.0.0.1", "0.0.0.0", "[::1]"];

/// Node-operator environment forwarded into every container on top of the
/// shared provider-credential allowlist ([`env::PROVIDER_ENV`]). Host paths
/// (`PATH`, `HOME`, …) deliberately stay on the node — the image defines its
/// own.
const FORWARDED_NODE_ENV: &[&str] = &["RUST_LOG"];

/// In-image path of the `claude` CLI (installed into the `buzz` user's home
/// by the `claude` variant of `Dockerfile.agent`). The Claude ACP adapter is
/// pointed at it through `CLAUDE_CODE_EXECUTABLE` — an in-image path, never a
/// host path, which is why the docker substrate does not reuse the process
/// substrate's host `claude` resolution.
const CONTAINER_CLAUDE_CLI: &str = "/home/buzz/.local/bin/claude";

/// Configuration for the Docker substrate.
#[derive(Debug, Clone)]
pub struct DockerSubstrateConfig {
    /// Node data directory. Short-lived env-files live under `env/` for the
    /// duration of a `docker run`.
    pub data_dir: PathBuf,
    /// Relay the node itself is connected to — the fallback relay for bodies
    /// whose agent context does not carry one.
    pub relay_url: String,
    /// Agent body image for workloads the slim image can run (built from
    /// `Dockerfile.agent`, e.g. via `just agent-image`): the bundled
    /// `buzz-agent` runtime and unknown runtime identifiers, which custom
    /// images may carry. Catalog runtimes with their own image variant
    /// (goose/claude/codex) run `<repository>:<runtime-id>` instead, where
    /// the repository is this image with its tag (or digest) stripped —
    /// e.g. `--image myrepo/buzz-agent:v3` resolves the goose runtime to
    /// `myrepo/buzz-agent:goose`.
    pub image: String,
    /// Docker CLI used for every daemon interaction.
    pub docker_path: PathBuf,
    /// Relay URL as reachable from inside containers. When absent, loopback
    /// relay hosts are rewritten to `host.docker.internal`.
    pub container_relay_url: Option<String>,
    /// Grace period `docker stop`/`docker restart` allow between SIGTERM and
    /// the SIGKILL escalation.
    pub graceful_stop: Duration,
}

impl DockerSubstrateConfig {
    /// Build a configuration with default docker lookup and stop behavior.
    pub fn new(data_dir: PathBuf, relay_url: impl Into<String>, image: impl Into<String>) -> Self {
        Self {
            data_dir,
            relay_url: relay_url.into(),
            image: image.into(),
            docker_path: PathBuf::from("docker"),
            container_relay_url: None,
            graceful_stop: Duration::from_secs(10),
        }
    }

    /// Resolve the relay URL a container should be handed: an explicit
    /// per-workload relay wins when containers can reach it; loopback relays
    /// become the configured container relay override, or the
    /// `host.docker.internal` rewrite when none is set.
    fn relay_url_for_containers(&self, agent_relay: Option<&str>) -> String {
        if let Some(relay) = agent_relay {
            return match rewrite_loopback(relay) {
                None => relay.to_string(),
                Some(rewritten) => self.container_relay_url.clone().unwrap_or(rewritten),
            };
        }
        if let Some(explicit) = &self.container_relay_url {
            return explicit.clone();
        }
        rewrite_loopback(&self.relay_url).unwrap_or_else(|| self.relay_url.clone())
    }

    /// Resolve the agent body image for one runtime's image variant.
    ///
    /// `None` (the slim image suffices, or the runtime is unknown and may
    /// live in a custom image) resolves to the configured image verbatim.
    /// `Some(variant)` resolves to `<repository>:<variant>`, where the
    /// repository is the configured image with its tag or digest stripped:
    /// the default `buzz-agent:local` yields `buzz-agent:goose`, an operator
    /// override `myrepo/buzz-agent:v3` yields `myrepo/buzz-agent:goose`.
    fn image_for(&self, variant: Option<&str>) -> String {
        let Some(variant) = variant else {
            return self.image.clone();
        };
        format!("{}:{variant}", image_repository(&self.image))
    }
}

/// Strip the tag or digest off an image reference, leaving the repository.
///
/// A trailing `:tag` is only a tag when it comes after the last `/` —
/// `registry:5000/buzz-agent` has no tag, only a registry port.
fn image_repository(image: &str) -> &str {
    let repository = image.split_once('@').map_or(image, |(repo, _)| repo);
    match repository.rsplit_once(':') {
        Some((repo, tag)) if !tag.contains('/') => repo,
        _ => repository,
    }
}

/// Owner-scoped substrate identity of one workload.
type BodyKey = (String, WorkloadId);

/// In-memory record of one armed `docker wait` watcher.
///
/// This is supervision bookkeeping only — the launch key lives in the
/// container, and the durable spec lives in the ledger.
#[derive(Debug)]
struct ContainerWatch {
    /// Monotonic watcher generation, so a stale watcher cannot clear the
    /// slot of a replacement container.
    generation: u64,
    /// Set before the substrate stops/removes a container so the watcher
    /// does not report a substrate-initiated exit as a self-exit.
    expected_exit: Arc<AtomicBool>,
}

/// Substrate that runs each workload body as a Docker container.
#[derive(Debug)]
pub struct DockerSubstrate {
    config: DockerSubstrateConfig,
    entries: Arc<Mutex<HashMap<BodyKey, ContainerWatch>>>,
    exit_tx: mpsc::UnboundedSender<WorkloadExit>,
    generations: AtomicU64,
}

impl DockerSubstrate {
    /// Create the substrate, failing fast when the Docker daemon is
    /// unreachable, and re-arm exit watchers for every running container
    /// carrying this substrate's workload labels.
    ///
    /// Returns the substrate and the channel on which it reports bodies that
    /// exited on their own (never exits the substrate caused itself).
    pub async fn connect(
        config: DockerSubstrateConfig,
    ) -> Result<(Self, mpsc::UnboundedReceiver<WorkloadExit>), SubstrateError> {
        let (exit_tx, exit_rx) = mpsc::unbounded_channel();
        let substrate = Self {
            config,
            entries: Arc::new(Mutex::new(HashMap::new())),
            exit_tx,
            generations: AtomicU64::new(0),
        };

        // Fail fast: a node that cannot reach the daemon must not announce a
        // docker substrate it cannot honor.
        let version = substrate
            .docker_output(&["version", "--format", "{{.Server.Version}}"], &[])
            .await?;
        if !version.status.success() {
            return Err(SubstrateError::new(
                SafeErrorCode::RuntimeUnavailable,
                format!(
                    "docker daemon unreachable via {}: {}",
                    substrate.config.docker_path.display(),
                    stderr_snippet(&version.stderr),
                ),
            ));
        }

        substrate.rearm_exit_watchers().await?;
        Ok((substrate, exit_rx))
    }

    /// Re-arm `docker wait` watchers for running containers labeled as this
    /// substrate's workloads — bodies that outlived a node restart.
    async fn rearm_exit_watchers(&self) -> Result<(), SubstrateError> {
        let listing = self
            .docker_output(
                &[
                    "ps",
                    "--filter",
                    &format!("label={LABEL_WORKLOAD}"),
                    "--format",
                    &format!("{{{{.Label \"{LABEL_OWNER}\"}}}}\t{{{{.Label \"{LABEL_WORKLOAD}\"}}}}\t{{{{.Names}}}}"),
                ],
                &[],
            )
            .await?;
        if !listing.status.success() {
            return Err(SubstrateError::new(
                SafeErrorCode::RuntimeUnavailable,
                format!(
                    "list running workload containers: {}",
                    stderr_snippet(&listing.stderr)
                ),
            ));
        }
        for line in String::from_utf8_lossy(&listing.stdout).lines() {
            let mut fields = line.split('\t');
            let (Some(owner), Some(workload), Some(name)) =
                (fields.next(), fields.next(), fields.next())
            else {
                continue;
            };
            let Ok(workload_id) = WorkloadId::new(workload) else {
                warn!(
                    container = name,
                    "ignoring workload container with an invalid workload label"
                );
                continue;
            };
            self.arm_exit_watcher((owner.to_string(), workload_id), name.to_string())
                .await;
        }
        Ok(())
    }

    /// Deterministic container name for one workload under one owner.
    fn container_name(owner: &str, workload_id: &WorkloadId) -> String {
        let owner_prefix: String = owner
            .chars()
            .filter(char::is_ascii_alphanumeric)
            .take(12)
            .collect();
        format!("buzz-agent-{owner_prefix}-{}", workload_id.as_str())
    }

    fn env_file_path(&self, container: &str) -> PathBuf {
        self.config
            .data_dir
            .join("env")
            .join(format!("{container}.env"))
    }

    /// Run the docker CLI with the given arguments. `client_env` sets extra
    /// variables on the docker *client* process — used for name-only `-e`
    /// pass-through so values never appear on argv.
    async fn docker_output(
        &self,
        args: &[&str],
        client_env: &[(String, Zeroizing<String>)],
    ) -> Result<std::process::Output, SubstrateError> {
        let mut command = tokio::process::Command::new(&self.config.docker_path);
        command.args(args);
        for (name, value) in client_env {
            command.env(name, value.as_str());
        }
        command.stdin(Stdio::null());
        command.output().await.map_err(|error| {
            SubstrateError::new(
                SafeErrorCode::RuntimeUnavailable,
                format!(
                    "run {} {}: {error}",
                    self.config.docker_path.display(),
                    args.first().unwrap_or(&"")
                ),
            )
        })
    }

    /// Mark the current watcher for `key` (if any) as expecting an exit, so
    /// a substrate-initiated stop/replace is not reported as a self-exit.
    async fn mark_expected_exit(&self, key: &BodyKey) {
        if let Some(watch) = self.entries.lock().await.get(key) {
            watch.expected_exit.store(true, Ordering::Release);
        }
    }

    /// Arm a `docker wait` watcher for one running container and register it
    /// under `key`, replacing any previous registration.
    async fn arm_exit_watcher(&self, key: BodyKey, container: String) {
        let generation = self.generations.fetch_add(1, Ordering::Relaxed) + 1;
        let expected_exit = Arc::new(AtomicBool::new(false));
        self.entries.lock().await.insert(
            key.clone(),
            ContainerWatch {
                generation,
                expected_exit: Arc::clone(&expected_exit),
            },
        );
        let docker_path = self.config.docker_path.clone();
        let entries = Arc::clone(&self.entries);
        let exit_tx = self.exit_tx.clone();
        tokio::spawn(async move {
            let output = tokio::process::Command::new(&docker_path)
                .args(["wait", &container])
                .stdin(Stdio::null())
                .output()
                .await;
            // `docker wait` prints the container's exit code; anything else
            // (daemon error, unparseable output) counts as an unclean end.
            let clean = matches!(
                &output,
                Ok(output) if output.status.success()
                    && String::from_utf8_lossy(&output.stdout).trim() == "0"
            );
            {
                let mut entries = entries.lock().await;
                if entries
                    .get(&key)
                    .is_some_and(|watch| watch.generation == generation)
                {
                    entries.remove(&key);
                }
            }
            if !expected_exit.load(Ordering::Acquire) {
                // The body exited on its own: it was finished, not killed.
                // Report it and never respawn ("Agents That Know When to
                // Leave").
                let _ = exit_tx.send(WorkloadExit {
                    owner: key.0,
                    workload_id: key.1,
                    clean,
                });
            }
        });
    }

    /// Recover the launch key from an existing container's environment — the
    /// container-as-key-store read path used by keyless redeploys.
    async fn recover_launch_key(
        &self,
        container: &str,
    ) -> Result<Option<Zeroizing<String>>, SubstrateError> {
        let output = self
            .docker_output(
                &[
                    "inspect",
                    "--format",
                    "{{range .Config.Env}}{{println .}}{{end}}",
                    container,
                ],
                &[],
            )
            .await?;
        if !output.status.success() {
            return Ok(None);
        }
        let environment = Zeroizing::new(String::from_utf8_lossy(&output.stdout).into_owned());
        Ok(environment
            .lines()
            .find_map(|line| line.strip_prefix("BUZZ_PRIVATE_KEY="))
            .map(|key| Zeroizing::new(key.to_string())))
    }

    /// Require the resolved agent body image to be present on the node,
    /// failing the deploy closed when it is not. The substrate never pulls
    /// or builds an image — the error tells the operator the exact
    /// `just agent-image` invocation that produces the missing variant.
    async fn require_local_image(
        &self,
        image: &str,
        variant: Option<&str>,
    ) -> Result<(), SubstrateError> {
        let inspect = self
            .docker_output(&["image", "inspect", image], &[])
            .await?;
        if inspect.status.success() {
            return Ok(());
        }
        let build_command = match variant {
            Some(variant) => format!("just agent-image {variant}"),
            None => "just agent-image".to_string(),
        };
        Err(SubstrateError::new(
            SafeErrorCode::RuntimeUnavailable,
            format!(
                "agent image {image} is not present on this node; build it on the node \
                 with `{build_command}` (Dockerfile.agent) and redeploy"
            ),
        ))
    }

    /// Replace the container for one workload: best-effort `rm -f`, then a
    /// fresh `docker run -d` with the harness environment delivered through a
    /// short-lived `0600` env-file (never argv).
    async fn run_container(
        &self,
        owner: &str,
        spec: &WorkloadSpec,
        launch_key: &Zeroizing<String>,
    ) -> Result<(), SubstrateError> {
        let agent = spec.agent.as_ref().ok_or_else(|| {
            SubstrateError::new(
                SafeErrorCode::Unsupported,
                "the docker substrate only runs managed-agent workloads",
            )
        })?;
        let resolved = container_runtime_launch(&spec.runtime);
        let image = self.config.image_for(resolved.image_variant);
        // Fail closed before the existing container is touched — it is the
        // key store, and a deploy that cannot run must not destroy it. The
        // node never pulls or builds images; the operator does, explicitly.
        self.require_local_image(&image, resolved.image_variant)
            .await?;
        let container = Self::container_name(owner, &spec.workload_id);
        let key: BodyKey = (owner.to_string(), spec.workload_id.clone());

        // The replaced container's death is substrate-initiated.
        self.mark_expected_exit(&key).await;
        // rm-before-run: converge to a single container per workload. A
        // missing container is fine; any other failure surfaces as a name
        // conflict on the run below.
        let _ = self.docker_output(&["rm", "-f", &container], &[]).await;

        let relay_url = self
            .config
            .relay_url_for_containers(agent.relay_url.as_deref());
        let mut environment = env::harness_environment(
            spec,
            agent,
            launch_key.as_str(),
            &relay_url,
            &resolved.launch,
        );
        if resolved.wants_claude_cli {
            environment.push((
                "CLAUDE_CODE_EXECUTABLE".to_string(),
                Zeroizing::new(CONTAINER_CLAUDE_CLI.to_string()),
            ));
        }
        // Node-operator provider credentials, forwarded — never workload
        // configuration.
        for name in FORWARDED_NODE_ENV.iter().chain(env::PROVIDER_ENV) {
            if let Ok(value) = std::env::var(name) {
                environment.push((name.to_string(), Zeroizing::new(value)));
            }
        }

        // An env-file line is `NAME=value` with no escaping, so values with
        // line breaks cannot travel in it. Those go as name-only `-e NAME`
        // flags with the value set on the docker client process — still never
        // on argv.
        let mut file_contents = Zeroizing::new(String::new());
        let mut passthrough: Vec<(String, Zeroizing<String>)> = Vec::new();
        for (name, value) in environment {
            if value.contains('\n') || value.contains('\r') {
                passthrough.push((name, value));
            } else {
                file_contents.push_str(&name);
                file_contents.push('=');
                file_contents.push_str(&value);
                file_contents.push('\n');
            }
        }
        let env_file = self.write_env_file(&container, &file_contents)?;

        let mut args: Vec<String> = vec![
            "run".into(),
            "-d".into(),
            "--init".into(),
            "--name".into(),
            container.clone(),
            "--label".into(),
            format!("{LABEL_OWNER}={owner}"),
            "--label".into(),
            format!("{LABEL_WORKLOAD}={}", spec.workload_id.as_str()),
            // Make the loopback rewrite work on native Linux engines too;
            // Docker Desktop resolves the name natively and ignores this.
            "--add-host".into(),
            format!("{HOST_GATEWAY_NAME}:host-gateway"),
            // The node owns lifecycle; the daemon must never respawn a body
            // that exited on its own.
            "--restart".into(),
            "no".into(),
            "--env-file".into(),
            env_file.path.display().to_string(),
        ];
        for (name, _) in &passthrough {
            args.push("-e".into());
            args.push(name.clone());
        }
        args.push(image.clone());

        let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let output = self.docker_output(&arg_refs, &passthrough).await?;
        drop(env_file); // Remove the env-file the moment the run returned.
        if !output.status.success() {
            return Err(SubstrateError::new(
                SafeErrorCode::RuntimeFailed,
                format!(
                    "docker run for image {image}: {}",
                    stderr_snippet(&output.stderr)
                ),
            ));
        }
        self.arm_exit_watcher(key, container).await;
        Ok(())
    }

    /// Write the env-file with `0600` permissions from the very first byte.
    fn write_env_file(
        &self,
        container: &str,
        contents: &str,
    ) -> Result<EnvFileGuard, SubstrateError> {
        let path = self.env_file_path(container);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                SubstrateError::new(
                    SafeErrorCode::RuntimeFailed,
                    format!("create env-file directory: {error}"),
                )
            })?;
        }
        // Remove any leftover file so create_new applies fresh 0600 perms and
        // never follows a pre-existing symlink.
        let _ = fs::remove_file(&path);
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&path).map_err(|error| {
            SubstrateError::new(
                SafeErrorCode::RuntimeFailed,
                format!("create env-file: {error}"),
            )
        })?;
        let guard = EnvFileGuard { path };
        use std::io::Write;
        file.write_all(contents.as_bytes()).map_err(|error| {
            SubstrateError::new(
                SafeErrorCode::RuntimeFailed,
                format!("write env-file: {error}"),
            )
        })?;
        Ok(guard)
    }

    fn missing_container_error() -> SubstrateError {
        SubstrateError::new(
            SafeErrorCode::RuntimeUnavailable,
            "no container holds this workload's launch key (it was removed from the \
             docker daemon); redeploy the agent from Desktop",
        )
    }
}

/// Deletes the short-lived env-file on every exit path.
struct EnvFileGuard {
    path: PathBuf,
}

impl Drop for EnvFileGuard {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_file(&self.path) {
            warn!(path = %self.path.display(), %error, "failed to remove short-lived env-file");
        }
    }
}

#[async_trait]
impl Substrate for DockerSubstrate {
    async fn deploy(&self, owner: &str, workload: &WorkloadSpec) -> Result<(), SubstrateError> {
        let agent = workload.agent.as_ref().ok_or_else(|| {
            SubstrateError::new(
                SafeErrorCode::Unsupported,
                "the docker substrate only runs managed-agent workloads",
            )
        })?;
        // One-time key handoff: prefer the key in this deploy, falling back
        // to the one stored in the existing container (the container is the
        // key store).
        let launch_key: Zeroizing<String> = match agent.private_key_nsec.clone() {
            Some(nsec) => Zeroizing::new(nsec),
            None => {
                let container = Self::container_name(owner, &workload.workload_id);
                self.recover_launch_key(&container).await?.ok_or_else(|| {
                    SubstrateError::new(
                        SafeErrorCode::InvalidCommand,
                        "deploy carries no launch key and no existing container holds one; \
                         redeploy the agent from Desktop",
                    )
                })?
            }
        };
        self.run_container(owner, workload, &launch_key).await
    }

    async fn start(&self, owner: &str, workload: &WorkloadSpec) -> Result<(), SubstrateError> {
        let key: BodyKey = (owner.to_string(), workload.workload_id.clone());
        if self.entries.lock().await.contains_key(&key) {
            // Idempotent: a watched container is already running.
            return Ok(());
        }
        let container = Self::container_name(owner, &workload.workload_id);
        let output = self.docker_output(&["start", &container], &[]).await?;
        if !output.status.success() {
            if is_no_such_container(&output.stderr) {
                // Fail closed: the container was the key store, and it is gone.
                return Err(Self::missing_container_error());
            }
            return Err(SubstrateError::new(
                SafeErrorCode::RuntimeFailed,
                format!("docker start: {}", stderr_snippet(&output.stderr)),
            ));
        }
        self.arm_exit_watcher(key, container).await;
        Ok(())
    }

    async fn stop(&self, owner: &str, workload_id: &WorkloadId) -> Result<(), SubstrateError> {
        let key: BodyKey = (owner.to_string(), workload_id.clone());
        self.mark_expected_exit(&key).await;
        let container = Self::container_name(owner, workload_id);
        let grace = self.config.graceful_stop.as_secs().to_string();
        let output = self
            .docker_output(&["stop", "-t", &grace, &container], &[])
            .await?;
        if !output.status.success() && !is_no_such_container(&output.stderr) {
            return Err(SubstrateError::new(
                SafeErrorCode::RuntimeFailed,
                format!("docker stop: {}", stderr_snippet(&output.stderr)),
            ));
        }
        // Deterministically clear the watcher slot so an immediate `start`
        // does not mistake the stopped container for a running one.
        self.entries.lock().await.remove(&key);
        Ok(())
    }

    async fn restart(&self, owner: &str, workload: &WorkloadSpec) -> Result<(), SubstrateError> {
        let key: BodyKey = (owner.to_string(), workload.workload_id.clone());
        self.mark_expected_exit(&key).await;
        let container = Self::container_name(owner, &workload.workload_id);
        let grace = self.config.graceful_stop.as_secs().to_string();
        let output = self
            .docker_output(&["restart", "-t", &grace, &container], &[])
            .await?;
        if !output.status.success() {
            if is_no_such_container(&output.stderr) {
                // Fail closed, same as start: the key store is gone.
                return Err(Self::missing_container_error());
            }
            return Err(SubstrateError::new(
                SafeErrorCode::RuntimeFailed,
                format!("docker restart: {}", stderr_snippet(&output.stderr)),
            ));
        }
        self.arm_exit_watcher(key, container).await;
        Ok(())
    }

    async fn remove(&self, owner: &str, workload_id: &WorkloadId) -> Result<(), SubstrateError> {
        let key: BodyKey = (owner.to_string(), workload_id.clone());
        self.mark_expected_exit(&key).await;
        let container = Self::container_name(owner, workload_id);
        let output = self.docker_output(&["rm", "-f", &container], &[]).await?;
        if !output.status.success() && !is_no_such_container(&output.stderr) {
            return Err(SubstrateError::new(
                SafeErrorCode::RuntimeFailed,
                format!("docker rm: {}", stderr_snippet(&output.stderr)),
            ));
        }
        self.entries.lock().await.remove(&key);
        // Removing the container removed the key store; scrub any env-file a
        // crashed deploy might have left behind.
        let _ = fs::remove_file(self.env_file_path(&container));
        Ok(())
    }
}

/// Container-side resolution of one runtime identifier: how the harness
/// launches it, and which agent body image variant carries it.
struct ContainerLaunch<'a> {
    /// Harness launch details (shared contract with the process substrate).
    launch: env::RuntimeLaunch<'a>,
    /// Whether to point the Claude adapter at [`CONTAINER_CLAUDE_CLI`].
    wants_claude_cli: bool,
    /// Image variant carrying the runtime; `None` runs the configured image.
    image_variant: Option<&'static str>,
}

/// Launch details for runtimes bundled in the agent body image variants.
///
/// Resolves against the shared runtime catalog ([`env::known_runtime`]) — the
/// same one the process substrate and the desktop launcher use. Unlike the
/// process substrate, nothing is resolved to a path here: each runtime's
/// image variant (`Dockerfile.agent`, `RUNTIME` build arg) bakes its tooling
/// onto the image's `PATH`, so command names travel as-is. Unknown runtime
/// identifiers are attempted verbatim inside the configured image so custom
/// images keep working; a missing command surfaces as the body's own launch
/// failure.
fn container_runtime_launch(runtime: &str) -> ContainerLaunch<'_> {
    let normalized = runtime.trim().to_ascii_lowercase();
    match env::known_runtime(&normalized) {
        Some(known) => ContainerLaunch {
            launch: env::RuntimeLaunch {
                agent_command: known.command,
                mcp_command: known.mcp,
                default_env: known.default_env,
                model_env: known.model_env,
                provider_env: known.provider_env,
            },
            wants_claude_cli: known.wants_claude_cli,
            image_variant: known.image_variant,
        },
        None => ContainerLaunch {
            launch: env::RuntimeLaunch {
                agent_command: runtime,
                mcp_command: None,
                default_env: &[],
                model_env: None,
                provider_env: None,
            },
            wants_claude_cli: false,
            image_variant: None,
        },
    }
}

/// Rewrite a loopback URL host to `host.docker.internal`, returning `None`
/// when the URL does not point at a loopback host.
fn rewrite_loopback(url: &str) -> Option<String> {
    let (scheme, rest) = url.split_once("://")?;
    for host in LOOPBACK_HOSTS {
        if let Some(tail) = rest.strip_prefix(host) {
            if tail.is_empty() || tail.starts_with(':') || tail.starts_with('/') {
                return Some(format!("{scheme}://{HOST_GATEWAY_NAME}{tail}"));
            }
        }
    }
    None
}

/// Whether a docker CLI failure means the named container does not exist —
/// the idempotent-success case for stop and remove.
fn is_no_such_container(stderr: &[u8]) -> bool {
    String::from_utf8_lossy(stderr)
        .to_ascii_lowercase()
        .contains("no such container")
}

/// Bounded, single-line stderr excerpt for node-local diagnostics.
fn stderr_snippet(stderr: &[u8]) -> String {
    let text = String::from_utf8_lossy(stderr);
    let mut snippet: String = text.trim().chars().take(300).collect();
    if snippet.is_empty() {
        snippet.push_str("(no diagnostic output)");
    }
    snippet.replace('\n', " | ")
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use buzz_core::execution::AgentWorkloadContext;
    use nostr::{Keys, ToBech32};
    use std::path::Path;
    use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static TEMP_DIR_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn temp_dir() -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock before epoch")
            .as_nanos();
        let counter = TEMP_DIR_COUNTER.fetch_add(1, AtomicOrdering::Relaxed);
        let dir = std::env::temp_dir().join(format!("buzz-node-docker-{suffix}-{counter}"));
        fs::create_dir_all(&dir).expect("test dir");
        dir
    }

    /// Hermetic docker stand-in: records every invocation, captures the
    /// env-file (with its permission bits) before the substrate deletes it,
    /// and simulates lifecycle behavior via marker files in the test dir.
    fn write_stub_docker(dir: &Path) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join("docker");
        let body = format!(
            r#"#!/bin/sh
DIR='{dir}'
printf '%s\n' "$*" >> "$DIR/calls.log"
cmd="$1"; shift
case "$cmd" in
  version)
    [ -f "$DIR/version.fail" ] && {{ echo "Cannot connect to the Docker daemon" >&2; exit 1; }}
    echo "29.0.0"
    ;;
  ps)
    [ -f "$DIR/ps.out" ] && cat "$DIR/ps.out"
    ;;
  image)
    shift
    if [ -f "$DIR/missing-images" ] && grep -qxF "$1" "$DIR/missing-images"; then
      echo "Error: No such image: $1" >&2
      exit 1
    fi
    echo "[]"
    ;;
  inspect)
    [ -f "$DIR/envfile.captured" ] || {{ echo "Error: No such container" >&2; exit 1; }}
    cat "$DIR/envfile.captured"
    ;;
  run)
    prev=""
    for arg in "$@"; do
      if [ "$prev" = "--env-file" ]; then
        cp "$arg" "$DIR/envfile.captured"
        if [ "$(uname)" = "Darwin" ]; then stat -f %Lp "$arg"; else stat -c %a "$arg"; fi > "$DIR/envfile.mode"
      fi
      prev="$arg"
    done
    if [ -n "$BUZZ_ACP_SYSTEM_PROMPT" ]; then printf '%s' "$BUZZ_ACP_SYSTEM_PROMPT" > "$DIR/system-prompt.captured"; fi
    echo "0123456789abcdef"
    ;;
  start)
    [ -f "$DIR/start.fail" ] && {{ echo "Error response from daemon: No such container" >&2; exit 1; }}
    ;;
  restart)
    [ -f "$DIR/restart.fail" ] && {{ echo "Error response from daemon: No such container" >&2; exit 1; }}
    ;;
  stop)
    [ -f "$DIR/stop.fail" ] && {{ echo "Error response from daemon: No such container" >&2; exit 1; }}
    : > "$DIR/stopped.flag"
    ;;
  rm)
    [ -f "$DIR/rm.fail" ] && {{ echo "Error: No such container" >&2; exit 1; }}
    ;;
  wait)
    if [ -f "$DIR/wait.code" ]; then cat "$DIR/wait.code"; exit 0; fi
    i=0
    while [ ! -f "$DIR/stopped.flag" ] && [ $i -lt 200 ]; do sleep 0.05; i=$((i+1)); done
    echo 137
    ;;
esac
exit 0
"#,
            dir = dir.display()
        );
        fs::write(&path, body).expect("write stub docker");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).expect("chmod stub docker");
        path
    }

    fn generated_agent_key() -> (String, String) {
        let keys = Keys::generate();
        (
            keys.secret_key().to_bech32().expect("nsec"),
            keys.public_key().to_hex(),
        )
    }

    fn agent_spec(nsec: Option<&str>, pubkey: &str) -> WorkloadSpec {
        let mut agent =
            AgentWorkloadContext::new(pubkey.to_string(), None, None, None, None, Vec::new(), None)
                .expect("agent context");
        if let Some(nsec) = nsec {
            agent = agent.with_private_key(nsec).expect("attach key");
        }
        let mut spec = WorkloadSpec::agent(
            WorkloadId::random(),
            "Docker substrate test agent",
            "buzz-agent",
            Some("test-model".to_string()),
            None,
            Vec::new(),
        )
        .expect("workload spec");
        spec.agent = Some(agent);
        spec
    }

    async fn connected_substrate(
        dir: &Path,
    ) -> (DockerSubstrate, mpsc::UnboundedReceiver<WorkloadExit>) {
        let stub = write_stub_docker(dir);
        let mut config = DockerSubstrateConfig::new(
            dir.to_path_buf(),
            "ws://localhost:3000".to_string(),
            "buzz-agent:test".to_string(),
        );
        config.docker_path = stub;
        config.graceful_stop = Duration::from_millis(300);
        DockerSubstrate::connect(config).await.expect("connect")
    }

    fn calls(dir: &Path) -> String {
        fs::read_to_string(dir.join("calls.log")).unwrap_or_default()
    }

    #[tokio::test]
    async fn connect_fails_fast_when_the_docker_daemon_is_unreachable() {
        let dir = temp_dir();
        let stub = write_stub_docker(&dir);
        fs::write(dir.join("version.fail"), "").expect("marker");
        let mut config = DockerSubstrateConfig::new(
            dir.clone(),
            "ws://localhost:3000".to_string(),
            "buzz-agent:test".to_string(),
        );
        config.docker_path = stub;
        let error = DockerSubstrate::connect(config)
            .await
            .expect_err("connect must fail fast");
        assert_eq!(error.code, SafeErrorCode::RuntimeUnavailable);
        assert!(error.message.contains("docker daemon unreachable"));
        let _ = fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn deploy_replaces_the_container_and_keeps_the_key_off_argv_in_a_0600_env_file() {
        let dir = temp_dir();
        // Keep the watcher's `docker wait` from blocking the test teardown.
        fs::write(dir.join("wait.code"), "0\n").expect("wait code");
        let (substrate, _exits) = connected_substrate(&dir).await;
        let (nsec, pubkey) = generated_agent_key();
        let spec = agent_spec(Some(&nsec), &pubkey);

        substrate.deploy("owner", &spec).await.expect("deploy");

        let log = calls(&dir);
        let rm_position = log.find("rm -f").expect("rm-before-run");
        let run_position = log.find("run -d").expect("docker run");
        assert!(rm_position < run_position, "deploy must rm before run");
        let run_line = log
            .lines()
            .find(|line| line.starts_with("run -d"))
            .expect("run line");
        assert!(run_line.contains(&format!(
            "--label buzz.node.workload={}",
            spec.workload_id.as_str()
        )));
        assert!(run_line.contains("--label buzz.node.owner=owner"));
        assert!(run_line.contains("--add-host host.docker.internal:host-gateway"));
        assert!(run_line.contains("--env-file"));
        assert!(run_line.ends_with("buzz-agent:test"));
        assert!(
            !log.contains(&nsec),
            "the launch key must never appear on docker argv"
        );

        let env_file = fs::read_to_string(dir.join("envfile.captured")).expect("captured env");
        assert!(env_file.contains(&format!("BUZZ_PRIVATE_KEY={nsec}")));
        assert!(
            env_file.contains("BUZZ_RELAY_URL=ws://host.docker.internal:3000"),
            "loopback relay must be rewritten for containers: {env_file}"
        );
        assert!(env_file.contains("BUZZ_ACP_AGENT_COMMAND=buzz-agent"));
        assert!(env_file.contains("BUZZ_ACP_MCP_COMMAND=buzz-dev-mcp"));
        assert!(env_file.contains("BUZZ_AGENT_MODEL=test-model"));

        let mode = fs::read_to_string(dir.join("envfile.mode")).expect("captured mode");
        assert_eq!(mode.trim(), "600", "env-file must be private");
        let leftovers: Vec<_> = fs::read_dir(dir.join("env"))
            .map(|entries| entries.flatten().collect())
            .unwrap_or_default();
        assert!(
            leftovers.is_empty(),
            "the env-file must be deleted once docker run returns"
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn deploy_passes_multiline_values_through_the_client_environment_not_argv() {
        let dir = temp_dir();
        fs::write(dir.join("wait.code"), "0\n").expect("wait code");
        let (substrate, _exits) = connected_substrate(&dir).await;
        let (nsec, pubkey) = generated_agent_key();
        let mut spec = agent_spec(Some(&nsec), &pubkey);
        let prompt = "You are helpful.\nAlways sign your work.";
        spec.agent.as_mut().expect("agent").system_prompt = Some(prompt.to_string());

        substrate.deploy("owner", &spec).await.expect("deploy");

        let run_line = calls(&dir)
            .lines()
            .find(|line| line.starts_with("run -d"))
            .expect("run line")
            .to_string();
        assert!(
            run_line.contains("-e BUZZ_ACP_SYSTEM_PROMPT"),
            "multiline values travel as name-only -e flags: {run_line}"
        );
        assert!(!run_line.contains("Always sign your work"));
        let env_file = fs::read_to_string(dir.join("envfile.captured")).expect("captured env");
        assert!(
            !env_file.contains("BUZZ_ACP_SYSTEM_PROMPT"),
            "an env-file line cannot carry a multiline value"
        );
        let captured =
            fs::read_to_string(dir.join("system-prompt.captured")).expect("captured prompt");
        assert_eq!(captured, prompt);
        let _ = fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn deploy_without_a_key_recovers_it_from_the_existing_container() {
        let dir = temp_dir();
        fs::write(dir.join("wait.code"), "0\n").expect("wait code");
        let (substrate, _exits) = connected_substrate(&dir).await;
        let (nsec, pubkey) = generated_agent_key();
        let spec = agent_spec(Some(&nsec), &pubkey);
        substrate.deploy("owner", &spec).await.expect("deploy");
        fs::remove_file(dir.join("calls.log")).expect("reset call log");

        // Redeploy without a key: the container is the key store.
        let redeploy = spec.clone().without_private_key();
        substrate
            .deploy("owner", &redeploy)
            .await
            .expect("keyless redeploy");
        let log = calls(&dir);
        assert!(log.contains("inspect"), "must read the key back: {log}");
        let env_file = fs::read_to_string(dir.join("envfile.captured")).expect("captured env");
        assert!(
            env_file.contains(&format!("BUZZ_PRIVATE_KEY={nsec}")),
            "the recovered key must reach the replacement container"
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn deploy_without_a_key_and_without_a_container_fails_closed() {
        let dir = temp_dir();
        let (substrate, _exits) = connected_substrate(&dir).await;
        let (_, pubkey) = generated_agent_key();
        let spec = agent_spec(None, &pubkey);

        let error = substrate
            .deploy("owner", &spec)
            .await
            .expect_err("keyless first deploy must fail");
        assert_eq!(error.code, SafeErrorCode::InvalidCommand);
        assert!(error.message.contains("redeploy"));
        let _ = fs::remove_dir_all(dir);
    }

    /// Deploy one runtime against the stub daemon and return the captured
    /// env-file contents and the `docker run` invocation line.
    async fn deploy_capture(runtime: &str) -> (String, String) {
        let dir = temp_dir();
        fs::write(dir.join("wait.code"), "0\n").expect("wait code");
        let (substrate, _exits) = connected_substrate(&dir).await;
        let (nsec, pubkey) = generated_agent_key();
        let mut spec = agent_spec(Some(&nsec), &pubkey);
        spec.runtime = runtime.to_string();
        substrate.deploy("owner", &spec).await.expect("deploy");
        let env_file = fs::read_to_string(dir.join("envfile.captured")).expect("captured env");
        let run_line = calls(&dir)
            .lines()
            .find(|line| line.starts_with("run -d"))
            .expect("run line")
            .to_string();
        let _ = fs::remove_dir_all(dir);
        (env_file, run_line)
    }

    #[tokio::test]
    async fn catalog_runtimes_map_to_variant_images_and_harness_env() {
        // Goose: the CLI itself is the ACP agent, non-interactive by default,
        // model via GOOSE_MODEL. No developer MCP. Runs its own image
        // variant, derived from the configured image's repository.
        let (goose, goose_run) = deploy_capture("goose").await;
        assert!(goose.contains("BUZZ_ACP_AGENT_COMMAND=goose\n"));
        assert!(goose.contains("BUZZ_ACP_MCP_COMMAND=\n"));
        assert!(goose.contains("GOOSE_MODE=auto\n"));
        assert!(goose.contains("GOOSE_MODEL=test-model\n"));
        assert!(goose_run.ends_with("buzz-agent:goose"), "{goose_run}");

        // Claude Code: the npm ACP adapter fronts the claude CLI, which the
        // adapter must find at its in-image path — never a host path.
        let (claude, claude_run) = deploy_capture("claude").await;
        assert!(claude.contains("BUZZ_ACP_AGENT_COMMAND=claude-agent-acp\n"));
        assert!(claude.contains("BUZZ_ACP_MCP_COMMAND=\n"));
        assert!(claude.contains(&format!("CLAUDE_CODE_EXECUTABLE={CONTAINER_CLAUDE_CLI}\n")));
        assert!(claude_run.ends_with("buzz-agent:claude"), "{claude_run}");

        // Runtime aliases resolve to the same image variant.
        let (_, claude_alias_run) = deploy_capture("claude-code").await;
        assert!(
            claude_alias_run.ends_with("buzz-agent:claude"),
            "{claude_alias_run}"
        );

        // Codex: npm ACP adapter plus the developer MCP.
        let (codex, codex_run) = deploy_capture("codex").await;
        assert!(codex.contains("BUZZ_ACP_AGENT_COMMAND=codex-acp\n"));
        assert!(codex.contains("BUZZ_ACP_MCP_COMMAND=buzz-dev-mcp\n"));
        assert!(!codex.contains("CLAUDE_CODE_EXECUTABLE="));
        assert!(codex_run.ends_with("buzz-agent:codex"), "{codex_run}");

        // Bundled buzz-agent: developer MCP plus its model env. The slim
        // configured image carries it.
        let (buzz_agent, buzz_agent_run) = deploy_capture("buzz-agent").await;
        assert!(buzz_agent.contains("BUZZ_ACP_AGENT_COMMAND=buzz-agent\n"));
        assert!(buzz_agent.contains("BUZZ_ACP_MCP_COMMAND=buzz-dev-mcp\n"));
        assert!(buzz_agent.contains("BUZZ_AGENT_MODEL=test-model\n"));
        assert!(
            buzz_agent_run.ends_with("buzz-agent:test"),
            "{buzz_agent_run}"
        );

        // Unknown runtimes are attempted verbatim inside the configured
        // image, so custom images keep working.
        let (custom, custom_run) = deploy_capture("my-custom-acp").await;
        assert!(custom.contains("BUZZ_ACP_AGENT_COMMAND=my-custom-acp\n"));
        assert!(custom.contains("BUZZ_ACP_MCP_COMMAND=\n"));
        assert!(!custom.contains("CLAUDE_CODE_EXECUTABLE="));
        assert!(custom_run.ends_with("buzz-agent:test"), "{custom_run}");
    }

    #[tokio::test]
    async fn deploy_fails_closed_when_the_resolved_image_is_missing() {
        let dir = temp_dir();
        let (substrate, _exits) = connected_substrate(&dir).await;
        fs::write(dir.join("missing-images"), "buzz-agent:goose\n").expect("marker");
        let (nsec, pubkey) = generated_agent_key();
        let mut spec = agent_spec(Some(&nsec), &pubkey);
        spec.runtime = "goose".to_string();

        let error = substrate
            .deploy("owner", &spec)
            .await
            .expect_err("deploy must fail closed on a missing image");
        assert_eq!(error.code, SafeErrorCode::RuntimeUnavailable);
        assert!(
            error.message.contains("buzz-agent:goose"),
            "{}",
            error.message
        );
        assert!(
            error.message.contains("just agent-image goose"),
            "the error must name the exact build command: {}",
            error.message
        );
        let log = calls(&dir);
        assert!(
            !log.contains("rm -f"),
            "a missing image must be detected before the existing container \
             (the key store) is removed: {log}"
        );
        assert!(!log.contains("run -d"), "{log}");

        // The slim configured image missing points at the default build.
        fs::write(dir.join("missing-images"), "buzz-agent:test\n").expect("marker");
        spec.runtime = "buzz-agent".to_string();
        let error = substrate
            .deploy("owner", &spec)
            .await
            .expect_err("deploy must fail closed on a missing base image");
        assert_eq!(error.code, SafeErrorCode::RuntimeUnavailable);
        assert!(
            error.message.contains("buzz-agent:test"),
            "{}",
            error.message
        );
        assert!(
            error.message.contains("`just agent-image`"),
            "{}",
            error.message
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn runtime_image_variants_derive_from_the_configured_repository() {
        let config = |image: &str| {
            DockerSubstrateConfig::new(PathBuf::from("/tmp"), "ws://localhost:3000", image)
        };
        // Default image: tag replaced by the variant id.
        assert_eq!(
            config("buzz-agent:local").image_for(Some("goose")),
            "buzz-agent:goose"
        );
        // Operator-overridden repository (and tag): repository kept, tag
        // replaced.
        assert_eq!(
            config("myrepo/buzz-agent:v3").image_for(Some("claude")),
            "myrepo/buzz-agent:claude"
        );
        // A registry port is not a tag.
        assert_eq!(
            config("registry.example:5000/buzz-agent").image_for(Some("codex")),
            "registry.example:5000/buzz-agent:codex"
        );
        // Digest pins are stripped the same way tags are.
        assert_eq!(
            config("buzz-agent@sha256:deadbeef").image_for(Some("goose")),
            "buzz-agent:goose"
        );
        assert_eq!(
            config("myrepo/buzz-agent:v3@sha256:deadbeef").image_for(Some("goose")),
            "myrepo/buzz-agent:goose"
        );
        // No variant: the configured image runs verbatim.
        assert_eq!(
            config("myrepo/buzz-agent:v3").image_for(None),
            "myrepo/buzz-agent:v3"
        );
    }

    #[tokio::test]
    async fn self_exit_is_reported_with_exit_status() {
        let dir = temp_dir();
        fs::write(dir.join("wait.code"), "0\n").expect("wait code");
        let (substrate, mut exits) = connected_substrate(&dir).await;
        let (nsec, pubkey) = generated_agent_key();
        let spec = agent_spec(Some(&nsec), &pubkey);
        substrate.deploy("owner", &spec).await.expect("deploy");
        let exit = tokio::time::timeout(Duration::from_secs(10), exits.recv())
            .await
            .expect("exit before timeout")
            .expect("exit event");
        assert_eq!(exit.owner, "owner");
        assert_eq!(exit.workload_id, spec.workload_id);
        assert!(exit.clean);

        let dir_unclean = temp_dir();
        fs::write(dir_unclean.join("wait.code"), "3\n").expect("wait code");
        let (substrate, mut exits) = connected_substrate(&dir_unclean).await;
        let spec = agent_spec(Some(&nsec), &pubkey);
        substrate.deploy("owner", &spec).await.expect("deploy");
        let exit = tokio::time::timeout(Duration::from_secs(10), exits.recv())
            .await
            .expect("exit before timeout")
            .expect("exit event");
        assert!(!exit.clean);
        let _ = fs::remove_dir_all(dir);
        let _ = fs::remove_dir_all(dir_unclean);
    }

    #[tokio::test]
    async fn stop_takes_the_container_down_without_reporting_a_self_exit() {
        let dir = temp_dir();
        let (substrate, mut exits) = connected_substrate(&dir).await;
        let (nsec, pubkey) = generated_agent_key();
        let spec = agent_spec(Some(&nsec), &pubkey);
        substrate.deploy("owner", &spec).await.expect("deploy");

        substrate
            .stop("owner", &spec.workload_id)
            .await
            .expect("stop");
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(
            exits.try_recv().is_err(),
            "a substrate-initiated stop must not surface as a self-exit"
        );
        // Stop is idempotent when nothing is running.
        substrate
            .stop("owner", &spec.workload_id)
            .await
            .expect("stop again");
        let _ = fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn stop_and_remove_treat_a_missing_container_as_success() {
        let dir = temp_dir();
        let (substrate, _exits) = connected_substrate(&dir).await;
        fs::write(dir.join("stop.fail"), "").expect("marker");
        fs::write(dir.join("rm.fail"), "").expect("marker");
        let workload_id = WorkloadId::random();

        substrate
            .stop("owner", &workload_id)
            .await
            .expect("stop of a missing container succeeds");
        substrate
            .remove("owner", &workload_id)
            .await
            .expect("remove of a missing container succeeds");
        let _ = fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn start_and_restart_fail_closed_once_the_container_is_gone() {
        let dir = temp_dir();
        let (substrate, _exits) = connected_substrate(&dir).await;
        fs::write(dir.join("start.fail"), "").expect("marker");
        fs::write(dir.join("restart.fail"), "").expect("marker");
        let (_, pubkey) = generated_agent_key();
        // The durable ledger spec never carries a key — exactly what a node
        // restart plus a lost container leaves behind.
        let spec = agent_spec(None, &pubkey);

        let error = substrate
            .start("owner", &spec)
            .await
            .expect_err("start must fail closed");
        assert_eq!(error.code, SafeErrorCode::RuntimeUnavailable);
        assert!(error.message.contains("redeploy"));

        let error = substrate
            .restart("owner", &spec)
            .await
            .expect_err("restart must fail closed");
        assert_eq!(error.code, SafeErrorCode::RuntimeUnavailable);
        assert!(error.message.contains("redeploy"));
        let _ = fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn exit_watchers_re_arm_from_labeled_containers_on_connect() {
        let dir = temp_dir();
        let workload_id = WorkloadId::random();
        let owner = "a1b2c3d4e5f6a1b2c3d4e5f6";
        fs::write(
            dir.join("ps.out"),
            format!(
                "{owner}\t{}\tbuzz-agent-a1b2c3d4e5f6-{}\n",
                workload_id.as_str(),
                workload_id.as_str()
            ),
        )
        .expect("ps listing");
        fs::write(dir.join("wait.code"), "0\n").expect("wait code");

        let (_substrate, mut exits) = connected_substrate(&dir).await;
        let exit = tokio::time::timeout(Duration::from_secs(10), exits.recv())
            .await
            .expect("exit before timeout")
            .expect("exit event");
        assert_eq!(exit.owner, owner);
        assert_eq!(exit.workload_id, workload_id);
        assert!(exit.clean);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn loopback_relay_urls_are_rewritten_for_containers() {
        assert_eq!(
            rewrite_loopback("ws://localhost:3000").as_deref(),
            Some("ws://host.docker.internal:3000")
        );
        assert_eq!(
            rewrite_loopback("ws://127.0.0.1:3000/path").as_deref(),
            Some("ws://host.docker.internal:3000/path")
        );
        assert_eq!(
            rewrite_loopback("wss://[::1]").as_deref(),
            Some("wss://host.docker.internal")
        );
        assert_eq!(rewrite_loopback("wss://relay.example"), None);
        assert_eq!(rewrite_loopback("wss://localhost.example"), None);
    }

    #[test]
    fn container_relay_resolution_prefers_reachable_relays_and_the_override() {
        let mut config = DockerSubstrateConfig::new(
            PathBuf::from("/tmp"),
            "ws://localhost:3000".to_string(),
            "buzz-agent:test".to_string(),
        );
        // No override: loopback node relay is rewritten.
        assert_eq!(
            config.relay_url_for_containers(None),
            "ws://host.docker.internal:3000"
        );
        // A non-loopback per-workload relay is honored verbatim.
        assert_eq!(
            config.relay_url_for_containers(Some("wss://relay.example")),
            "wss://relay.example"
        );
        // The override replaces unreachable loopback relays.
        config.container_relay_url = Some("wss://relay.corp.internal".to_string());
        assert_eq!(
            config.relay_url_for_containers(None),
            "wss://relay.corp.internal"
        );
        assert_eq!(
            config.relay_url_for_containers(Some("ws://127.0.0.1:3000")),
            "wss://relay.corp.internal"
        );
        assert_eq!(
            config.relay_url_for_containers(Some("wss://relay.example")),
            "wss://relay.example"
        );
    }
}
