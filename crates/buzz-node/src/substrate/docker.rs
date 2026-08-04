//! Docker substrate: runs each workload body as a Docker container.
//!
//! A deploy replaces any previous container for the workload (`docker rm -f`,
//! then `docker run -d`) using an agent body image with `buzz-acp` as the
//! entrypoint. The base image defaults to the published, digest-pinned
//! buzz-sprig image ([`DEFAULT_AGENT_IMAGE`] — the body holds an nsec, so the
//! default is immutable by construction, docs/remote-agents.md §Image); it
//! serves the bundled `buzz-agent` runtime and unknown runtimes, and remains
//! fully overridable via [`DockerSubstrateConfig::image`] (tag, digest, or
//! custom registry — the operator's trust decision, e.g. a local
//! `Dockerfile.agent` build). Each catalog runtime of [`env::known_runtime`]
//! with its own tooling — Goose, the Claude Code and Codex CLIs with their
//! npm ACP adapters — resolves instead from a dedicated local variant
//! repository ([`DockerSubstrateConfig::variant_image_repo`], default
//! `buzz-agent`) as `<repo>:<runtime>`, built on the node via
//! `just agent-image <runtime>` (`Dockerfile.agent`, `RUNTIME` build arg) —
//! the spec's "buzz-sprig plus your tools" override images. The substrate
//! fails a deploy closed when the resolved image is not present on the node —
//! it never pulls or builds. The harness environment contract is shared with
//! the process substrate ([`super::env`]); it reaches the container through
//! a short-lived `0600` env-file (plus name-only `-e` pass-through for
//! values an env-file cannot carry), never through command-line arguments.
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

/// Default agent body image: the published sprig agent-body image, pinned by
/// digest because the body holds an nsec — a tag is a movable pointer, a
/// digest is not (docs/remote-agents.md §Image). The tag stays in the
/// reference so the image remains human-traceable to its git SHA while the
/// digest does the pinning.
///
/// Keep in sync with crates/buzz-backend-kubernetes/src/config.rs
/// `DEFAULT_IMAGE` (the Kubernetes binding pins the same image; a unit test
/// asserts the two constants match so drift fails the build).
pub const DEFAULT_AGENT_IMAGE: &str = "ghcr.io/block/buzz-sprig:sha-6530b58@sha256:17facfc7608d8ddb33bc056c9aaba1098f4ef6abe5655702fbfd7584d1f74d76";

/// Default local repository for per-runtime agent image variants
/// (`<repo>:<runtime>`), matching what `just agent-image <runtime>` tags
/// (`Dockerfile.agent`).
pub const DEFAULT_VARIANT_IMAGE_REPO: &str = "buzz-agent";

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

/// Bound on the liveness confirmation a container must pass before a
/// deploy/start/restart is reported as succeeded.
///
/// `docker run -d` exiting zero only means the daemon accepted the container:
/// a body that dies immediately (missing provider credential, unusable
/// runtime) would otherwise earn a `Succeeded` receipt. A container that is
/// already running answers on the first poll, so this bound is only ever paid
/// by a body that is failing. The `docker wait` watcher stays the
/// after-the-fact path for everything that dies later.
const LIVENESS_TIMEOUT: Duration = Duration::from_millis(2000);

/// Gap between `docker inspect` liveness polls.
const LIVENESS_POLL: Duration = Duration::from_millis(100);

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
    /// Base agent body image for the bundled `buzz-agent` runtime and
    /// unknown runtime identifiers (which custom images may carry). Defaults
    /// to the digest-pinned published sprig image
    /// ([`DEFAULT_AGENT_IMAGE`]); overridable with any tag, digest, or
    /// custom registry reference — the operator's trust decision (e.g.
    /// `buzz-agent:local` from a local `just agent-image` build). Catalog
    /// runtimes with their own image variant (goose/claude/codex) resolve
    /// from [`Self::variant_image_repo`] instead, never from this image.
    pub image: String,
    /// Local repository for per-runtime agent image variants: catalog
    /// runtimes with an `image_variant` run `<repo>:<runtime>` (e.g.
    /// `buzz-agent:goose`), built on the node via
    /// `just agent-image <runtime>` (`Dockerfile.agent`). Defaults to
    /// [`DEFAULT_VARIANT_IMAGE_REPO`].
    pub variant_image_repo: String,
    /// Docker CLI used for every daemon interaction.
    pub docker_path: PathBuf,
    /// Relay URL as reachable from inside containers. When absent, loopback
    /// relay hosts are rewritten to `host.docker.internal`.
    pub container_relay_url: Option<String>,
    /// Grace period `docker stop`/`docker restart` allow between SIGTERM and
    /// the SIGKILL escalation.
    pub graceful_stop: Duration,
    /// Inactivity budget handed to every body as
    /// `BUZZ_ACP_EXIT_AFTER_INACTIVITY` (docs/remote-agents.md §Auto-Stop).
    /// `0` is the legal "no inactivity bound" and omits the variable — the
    /// harness default already means disabled.
    pub inactivity_seconds: u64,
}

impl DockerSubstrateConfig {
    /// Build a configuration with default docker lookup, stop behavior,
    /// variant image repository ([`DEFAULT_VARIANT_IMAGE_REPO`]), and
    /// inactivity budget ([`super::DEFAULT_INACTIVITY_SECONDS`]).
    pub fn new(data_dir: PathBuf, relay_url: impl Into<String>, image: impl Into<String>) -> Self {
        Self {
            data_dir,
            relay_url: relay_url.into(),
            image: image.into(),
            variant_image_repo: DEFAULT_VARIANT_IMAGE_REPO.to_string(),
            docker_path: PathBuf::from("docker"),
            container_relay_url: None,
            graceful_stop: Duration::from_secs(10),
            inactivity_seconds: super::DEFAULT_INACTIVITY_SECONDS,
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
    /// `None` (the base image suffices, or the runtime is unknown and may
    /// live in a custom image) resolves to the configured base image
    /// verbatim — including a digest-pinned default. `Some(variant)`
    /// resolves to `<variant_image_repo>:<variant>` (the default repository
    /// yields `buzz-agent:goose`), never from the base image: deriving
    /// variant tags from a digest-pinned reference would point at tags that
    /// do not exist on the registry.
    fn image_for(&self, variant: Option<&str>) -> String {
        match variant {
            None => self.image.clone(),
            Some(variant) => format!("{}:{variant}", self.variant_image_repo),
        }
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

    /// Confirm a just-launched container is actually running, polling
    /// `docker inspect` within [`LIVENESS_TIMEOUT`].
    ///
    /// A container that fails this check is deliberately left in place: it is
    /// still the workload's key store, and `docker logs` on it is the only
    /// diagnostic the operator has. Nothing is armed for it, so the substrate
    /// treats it as not running — a later `start` retries that same container
    /// rather than mistaking it for a live body.
    async fn confirm_running(&self, container: &str) -> Result<(), SubstrateError> {
        let deadline = tokio::time::Instant::now() + LIVENESS_TIMEOUT;
        let mut state: String;
        loop {
            let output = self
                .docker_output(
                    &["inspect", "--format", "{{.State.Running}}", container],
                    &[],
                )
                .await?;
            if output.status.success() {
                state = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if state == "true" {
                    return Ok(());
                }
            } else {
                state = stderr_snippet(&output.stderr);
            }
            if tokio::time::Instant::now() >= deadline {
                break;
            }
            tokio::time::sleep(LIVENESS_POLL).await;
        }
        Err(SubstrateError::new(
            SafeErrorCode::RuntimeFailed,
            format!(
                "container {container} is not running shortly after launch \
                 (docker reported {state:?}); inspect it with `docker logs {container}`"
            ),
        ))
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
    /// or builds an image (launch stages an exact artifact) — the error
    /// tells the operator the one exact command that produces it: `docker
    /// pull` of the full configured reference for the base image, the
    /// `just agent-image <runtime>` build for a variant image.
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
        let remedy = match variant {
            Some(variant) => {
                format!("build it on the node with `just agent-image {variant}` (Dockerfile.agent)")
            }
            None => format!("pull it on the node with `docker pull {image}`"),
        };
        Err(SubstrateError::new(
            SafeErrorCode::RuntimeUnavailable,
            format!("agent image {image} is not present on this node; {remedy} and redeploy"),
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
        // Command names from the contract run as-is: the image's PATH
        // carries them (see `container_runtime_launch`).
        let commands = env::ResolvedCommands {
            agent_command: &spec.launch.command,
            mcp_command: spec.launch.mcp_command.as_deref(),
        };
        let mut environment = env::harness_environment(
            spec,
            agent,
            owner,
            launch_key.as_str(),
            &relay_url,
            &commands,
            self.config.inactivity_seconds,
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
        self.confirm_running(&container).await?;
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
        self.confirm_running(&container).await?;
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
        self.confirm_running(&container).await?;
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

/// Container-side adaptation of one runtime identifier: whether the adapter
/// needs the in-image `claude` CLI, and which agent body image variant
/// carries the runtime.
///
/// Unlike the process substrate, nothing is resolved to a path here: each
/// runtime's image variant (`Dockerfile.agent`, `RUNTIME` build arg) bakes
/// its tooling onto the image's `PATH`, so the launch contract's command
/// names travel as-is. Unknown runtime identifiers run inside the configured
/// image so custom images keep working; a missing command surfaces as the
/// body's own launch failure.
fn container_runtime_launch(runtime: &str) -> env::KnownRuntime {
    env::known_runtime(&runtime.trim().to_ascii_lowercase())
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
    case "$2" in
      *State.Running*)
        if [ -f "$DIR/notrunning.flag" ]; then echo false; else echo true; fi
        ;;
      *)
        [ -f "$DIR/envfile.captured" ] || {{ echo "Error: No such container" >&2; exit 1; }}
        cat "$DIR/envfile.captured"
        ;;
    esac
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

    /// A resolved launch contract shaped like what Desktop produces for the
    /// bundled `buzz-agent` runtime: developer MCP plus its model policy env.
    fn test_launch(command: &str) -> buzz_core::execution::LaunchSpec {
        buzz_core::execution::LaunchSpec::new(
            command,
            Vec::new(),
            Some("buzz-dev-mcp".to_string()),
            std::collections::BTreeMap::new(),
            std::collections::BTreeMap::from([(
                "BUZZ_AGENT_MODEL".to_string(),
                "test-model".to_string(),
            )]),
            Some(nostr::Keys::generate().public_key().to_hex()),
        )
        .expect("launch contract")
    }

    fn agent_spec(nsec: Option<&str>, pubkey: &str) -> WorkloadSpec {
        let mut agent =
            AgentWorkloadContext::new(pubkey.to_string(), None, None, None).expect("agent context");
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
            test_launch("buzz-agent"),
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
    async fn deploy_fails_when_the_container_is_not_running_shortly_after_launch() {
        let dir = temp_dir();
        let (substrate, mut exits) = connected_substrate(&dir).await;
        // The daemon accepted `docker run -d`, but the body died immediately.
        fs::write(dir.join("notrunning.flag"), "").expect("marker");
        let (nsec, pubkey) = generated_agent_key();
        let spec = agent_spec(Some(&nsec), &pubkey);

        let error = substrate
            .deploy("owner", &spec)
            .await
            .expect_err("deploy must not succeed before the body is known to run");
        assert_eq!(error.code, SafeErrorCode::RuntimeFailed);
        assert!(
            error.message.contains(&format!(
                "docker logs buzz-agent-owner-{}",
                spec.workload_id.as_str()
            )),
            "the node-local message must point at the container's logs: {}",
            error.message
        );
        let log = calls(&dir);
        assert!(log.contains("run -d"), "{log}");
        // Nothing is armed for a container that never came up, so the
        // substrate does not mistake it for a live body.
        assert!(substrate.entries.lock().await.is_empty());
        assert!(
            exits.try_recv().is_err(),
            "a failed deploy reports through its receipt, not the exit channel"
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
        spec.launch
            .policy_env
            .insert("BUZZ_ACP_SYSTEM_PROMPT".to_string(), prompt.to_string());

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
    async fn deploy_capture(runtime: &str, command: &str, mcp: Option<&str>) -> (String, String) {
        let dir = temp_dir();
        fs::write(dir.join("wait.code"), "0\n").expect("wait code");
        let (substrate, _exits) = connected_substrate(&dir).await;
        let (nsec, pubkey) = generated_agent_key();
        let mut spec = agent_spec(Some(&nsec), &pubkey);
        spec.runtime = runtime.to_string();
        spec.launch.command = command.to_string();
        spec.launch.mcp_command = mcp.map(str::to_string);
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
    async fn runtime_identifiers_map_to_variant_images_and_the_contract_supplies_commands() {
        // The node no longer derives commands or env from the runtime
        // identifier: the launch contract carries them, and the identifier
        // only selects the agent-body image variant (plus the Claude CLI
        // pointer, a substrate adaptation concern).

        // Goose: its own image variant; the contract's command travels as-is.
        let (goose, goose_run) = deploy_capture("goose", "goose", None).await;
        assert!(goose.contains("BUZZ_ACP_AGENT_COMMAND=goose\n"));
        assert!(goose.contains("BUZZ_ACP_MCP_COMMAND=\n"));
        assert!(goose_run.ends_with("buzz-agent:goose"), "{goose_run}");

        // Claude Code: the npm ACP adapter fronts the claude CLI, which the
        // adapter must find at its in-image path — never a host path.
        let (claude, claude_run) = deploy_capture("claude", "claude-agent-acp", None).await;
        assert!(claude.contains("BUZZ_ACP_AGENT_COMMAND=claude-agent-acp\n"));
        assert!(claude.contains(&format!("CLAUDE_CODE_EXECUTABLE={CONTAINER_CLAUDE_CLI}\n")));
        assert!(claude_run.ends_with("buzz-agent:claude"), "{claude_run}");

        // Runtime aliases resolve to the same image variant.
        let (_, claude_alias_run) = deploy_capture("claude-code", "claude-agent-acp", None).await;
        assert!(
            claude_alias_run.ends_with("buzz-agent:claude"),
            "{claude_alias_run}"
        );

        // Codex: npm ACP adapter plus the developer MCP from the contract.
        let (codex, codex_run) = deploy_capture("codex", "codex-acp", Some("buzz-dev-mcp")).await;
        assert!(codex.contains("BUZZ_ACP_AGENT_COMMAND=codex-acp\n"));
        assert!(codex.contains("BUZZ_ACP_MCP_COMMAND=buzz-dev-mcp\n"));
        assert!(!codex.contains("CLAUDE_CODE_EXECUTABLE="));
        assert!(codex_run.ends_with("buzz-agent:codex"), "{codex_run}");

        // Bundled buzz-agent: the slim configured image carries it; policy
        // env (its model variable) travels in the contract, not the catalog.
        let (buzz_agent, buzz_agent_run) =
            deploy_capture("buzz-agent", "buzz-agent", Some("buzz-dev-mcp")).await;
        assert!(buzz_agent.contains("BUZZ_ACP_AGENT_COMMAND=buzz-agent\n"));
        assert!(buzz_agent.contains("BUZZ_ACP_MCP_COMMAND=buzz-dev-mcp\n"));
        assert!(buzz_agent.contains("BUZZ_AGENT_MODEL=test-model\n"));
        assert!(
            buzz_agent_run.ends_with("buzz-agent:test"),
            "{buzz_agent_run}"
        );

        // Unknown runtimes run the configured image with the contract's
        // command verbatim, so custom images keep working.
        let (custom, custom_run) = deploy_capture("my-custom-acp", "my-custom-acp", None).await;
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

        // The base image missing points at the exact pull of the full
        // configured reference — the one narrow path that stages it.
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
            error.message.contains("`docker pull buzz-agent:test`"),
            "the error must name the exact pull command for the full \
             configured reference: {}",
            error.message
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn inactivity_bound_reaches_the_env_file_and_zero_omits_it() {
        // Default configuration: the node's remote-body opt-in (7200s).
        let dir = temp_dir();
        fs::write(dir.join("wait.code"), "0\n").expect("wait code");
        let (substrate, _exits) = connected_substrate(&dir).await;
        assert_eq!(
            substrate.config.inactivity_seconds,
            crate::substrate::DEFAULT_INACTIVITY_SECONDS
        );
        let (nsec, pubkey) = generated_agent_key();
        let spec = agent_spec(Some(&nsec), &pubkey);
        substrate.deploy("owner", &spec).await.expect("deploy");
        let env_file = fs::read_to_string(dir.join("envfile.captured")).expect("captured env");
        assert!(
            env_file.contains("BUZZ_ACP_EXIT_AFTER_INACTIVITY=7200\n"),
            "{env_file}"
        );
        let _ = fs::remove_dir_all(&dir);

        // 0 is the legal "no inactivity bound": the variable is omitted so
        // the harness default (disabled) stays the single source of truth.
        let dir = temp_dir();
        fs::write(dir.join("wait.code"), "0\n").expect("wait code");
        let stub = write_stub_docker(&dir);
        let mut config = DockerSubstrateConfig::new(
            dir.clone(),
            "ws://localhost:3000".to_string(),
            "buzz-agent:test".to_string(),
        );
        config.docker_path = stub;
        config.inactivity_seconds = 0;
        let (substrate, _exits) = DockerSubstrate::connect(config).await.expect("connect");
        let spec = agent_spec(Some(&nsec), &pubkey);
        substrate.deploy("owner", &spec).await.expect("deploy");
        let env_file = fs::read_to_string(dir.join("envfile.captured")).expect("captured env");
        assert!(
            !env_file.contains("BUZZ_ACP_EXIT_AFTER_INACTIVITY"),
            "0 must omit the variable entirely: {env_file}"
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn runtime_image_variants_resolve_from_the_dedicated_variant_repository() {
        let config = |image: &str| {
            DockerSubstrateConfig::new(PathBuf::from("/tmp"), "ws://localhost:3000", image)
        };
        // Variants come from the variant repository, never from the base
        // image — the digest-pinned default has no goose/claude/codex tags
        // on its registry.
        assert_eq!(
            config(DEFAULT_AGENT_IMAGE).image_for(Some("goose")),
            "buzz-agent:goose"
        );
        assert_eq!(
            config("myrepo/buzz-agent:v3").image_for(Some("claude")),
            "buzz-agent:claude"
        );
        // The variant repository itself is configurable.
        let mut custom = config(DEFAULT_AGENT_IMAGE);
        custom.variant_image_repo = "registry.example:5000/buzz-agent".to_string();
        assert_eq!(
            custom.image_for(Some("codex")),
            "registry.example:5000/buzz-agent:codex"
        );
        // No variant: the configured base image runs verbatim — tag, digest,
        // or custom registry, including the digest-pinned default.
        assert_eq!(
            config(DEFAULT_AGENT_IMAGE).image_for(None),
            DEFAULT_AGENT_IMAGE
        );
        assert_eq!(
            config("myrepo/buzz-agent:v3@sha256:deadbeef").image_for(None),
            "myrepo/buzz-agent:v3@sha256:deadbeef"
        );
        assert_eq!(
            config("buzz-agent:local").image_for(None),
            "buzz-agent:local"
        );
    }

    /// The default image must stay byte-identical to the Kubernetes
    /// binding's `DEFAULT_IMAGE` — both pin the same published sprig image
    /// by digest. `buzz-backend-kubernetes` does not depend on `buzz-core`,
    /// so the constant cannot live in a shared crate; this test makes drift
    /// a test failure instead of a silent divergence.
    #[test]
    fn default_image_matches_the_kubernetes_backend_pin() {
        let source = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../buzz-backend-kubernetes/src/config.rs"
        ));
        let pinned = source
            .lines()
            .find_map(|line| line.strip_prefix("pub const DEFAULT_IMAGE: &str = \""))
            .and_then(|rest| rest.split('"').next())
            .expect("DEFAULT_IMAGE constant in buzz-backend-kubernetes/src/config.rs");
        assert_eq!(
            DEFAULT_AGENT_IMAGE, pinned,
            "buzz-node's DEFAULT_AGENT_IMAGE drifted from the Kubernetes \
             binding's DEFAULT_IMAGE; update both together"
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
