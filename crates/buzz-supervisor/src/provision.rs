//! Core reconciliation loop: discover channels, provision new ones whose
//! `description` declares a `workdir:`, heal dead role processes, and tear
//! down channels that disappear (archived/deleted).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use buzz_cli::client::{extract_d_tag, extract_tag_value, BuzzClient};
use buzz_core::kind::KIND_AGENT_PROFILE;
use nostr::{EventBuilder, Keys, Kind};
use regex::Regex;
use uuid::Uuid;

use crate::config::SupervisorConfig;
use crate::security::AllowedRoots;
use crate::state::{ChannelState, ChannelStatus, RoleState, StateStore};

pub struct Ctx {
    pub client: BuzzClient,
    pub config: SupervisorConfig,
    pub allowed_roots: AllowedRoots,
    pub state: StateStore,
    pub acp_bin: PathBuf,
    pub admin_bin: PathBuf,
    pub relay_admin_key: String,
    pub relay_url_for_admin: String,
    pub workdir_pattern: Regex,
}

/// One full pass over every channel: provision new matches, heal known
/// teams, tear down ones that vanished from the channel list.
pub async fn run_once(ctx: &Ctx) -> anyhow::Result<()> {
    let channels = list_channels(&ctx.client).await?;
    let seen: std::collections::HashSet<&str> =
        channels.iter().map(|c| c.channel_id.as_str()).collect();

    for channel in &channels {
        match ctx.state.load(&channel.channel_id)? {
            None => handle_new_channel(ctx, channel).await?,
            // Resume rather than skip: `provision_team` is safe to re-enter
            // (each step is keyed by role name / presence of a pid), so an
            // interruption on the previous poll just continues here instead
            // of sitting stuck forever.
            Some(state) if matches!(state.status, ChannelStatus::Provisioning) => {
                let workdir = state
                    .workdir
                    .clone()
                    .ok_or_else(|| anyhow::anyhow!("Provisioning state missing workdir"))?;
                provision_team(ctx, &channel.channel_id, Path::new(&workdir)).await?
            }
            Some(state) => heal_channel(ctx, &channel.channel_id, &state)?,
        }
    }

    for known_id in ctx.state.known_channels()? {
        if seen.contains(known_id.as_str()) {
            continue;
        }
        if let Some(state) = ctx.state.load(&known_id)? {
            if matches!(state.status, ChannelStatus::Provisioned) {
                tear_down(ctx, &known_id, &state)?;
            }
        }
    }

    Ok(())
}

struct ChannelSummary {
    channel_id: String,
    description: String,
}

async fn list_channels(client: &BuzzClient) -> anyhow::Result<Vec<ChannelSummary>> {
    let filter = serde_json::json!({ "kinds": [39000], "limit": 500 });
    let resp = client
        .query(&filter)
        .await
        .map_err(|e| anyhow::anyhow!("channels query failed: {e}"))?;
    let events: Vec<serde_json::Value> = serde_json::from_str(&resp)?;
    Ok(events
        .iter()
        .map(|e| ChannelSummary {
            channel_id: extract_d_tag(e),
            description: extract_tag_value(e, "about"),
        })
        .filter(|c| !c.channel_id.is_empty())
        .collect())
}

async fn handle_new_channel(ctx: &Ctx, channel: &ChannelSummary) -> anyhow::Result<()> {
    let Some(captures) = ctx.workdir_pattern.captures(&channel.description) else {
        ctx.state.save(
            &channel.channel_id,
            &ChannelState {
                status: ChannelStatus::Ignored,
                workdir: None,
                roles: HashMap::new(),
            },
        )?;
        return Ok(());
    };
    let raw_path = &captures[1];

    match ctx.allowed_roots.validate(raw_path) {
        Ok(resolved) => provision_team(ctx, &channel.channel_id, &resolved).await,
        Err(reason) => reject_channel(ctx, &channel.channel_id, &reason).await,
    }
}

/// Provisions a team, saving state to disk after every side-effecting step.
///
/// This is deliberately not "do everything, then save once at the end": an
/// earlier version did that, and any error partway through (e.g. the 3rd
/// role's `publish-profile` HTTP call failing) left the channel with no
/// state file at all — so the *next* poll saw it as brand new and started
/// over, generating and registering a fresh set of keys every poll interval
/// forever. Saving after each step means a mid-provisioning failure is
/// recorded as `Provisioning` with exactly the roles created so far, and is
/// never silently retried from scratch.
async fn provision_team(ctx: &Ctx, channel_id: &str, workdir: &Path) -> anyhow::Result<()> {
    let channel_uuid = Uuid::parse_str(channel_id)?;
    let mut state = ctx.state.load(channel_id)?.unwrap_or(ChannelState {
        status: ChannelStatus::Provisioning,
        workdir: Some(workdir.display().to_string()),
        roles: HashMap::new(),
    });

    let owner_pubkey = ctx.client.keys().public_key().to_hex();
    let mut allowlist: Vec<String> = ctx.config.extra_allowlist.clone();
    allowlist.push(owner_pubkey);

    // Phase 1: one identity per role — generate, register, add as a channel
    // member. Persisted immediately per-role so a failure here never
    // re-generates roles that already succeeded.
    for role in &ctx.config.roles {
        if state.roles.contains_key(&role.name) {
            continue; // already done in a prior (interrupted) attempt
        }
        let keys = Keys::generate();
        let pubkey = keys.public_key().to_hex();
        admin_register_member(ctx, &pubkey)?;
        add_channel_member(ctx, channel_uuid, &pubkey, "bot").await?;
        allowlist.push(pubkey.clone());
        state.roles.insert(
            role.name.clone(),
            RoleState {
                pubkey,
                privkey: keys.secret_key().display_secret().to_string(),
                pid: None,
            },
        );
        ctx.state.save(channel_id, &state)?;
    }
    for shared in &ctx.config.shared_members {
        add_channel_member(ctx, channel_uuid, shared, "bot").await?;
    }

    let project_label = workdir
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| workdir.display().to_string());

    // Phase 2: publish each role's profile and spawn its process. Also
    // persisted per-role, and skipped for roles that already have a pid
    // (i.e. already finished in a prior attempt).
    for role_config in &ctx.config.roles {
        let role_state = state
            .roles
            .get(&role_config.name)
            .ok_or_else(|| anyhow::anyhow!("role {} missing after phase 1", role_config.name))?
            .clone();
        if role_state.pid.is_some() {
            continue;
        }
        let keys = Keys::parse(&role_state.privkey)?;
        let display_name = format!("{} ({project_label})", role_config.display_name);

        publish_agent_profile(ctx, &keys, &display_name, channel_id, &allowlist).await?;
        publish_kind0_profile(ctx, &keys, &display_name).await?;

        let pid = spawn_role(ctx, channel_id, role_config, &keys, workdir, &allowlist)?;
        state.roles.insert(
            role_config.name.clone(),
            RoleState {
                pid: Some(pid),
                ..role_state
            },
        );
        ctx.state.save(channel_id, &state)?;
    }

    state.status = ChannelStatus::Provisioned;
    ctx.state.save(channel_id, &state)?;

    send_message(
        ctx,
        channel_uuid,
        &format!(
            "✅ Team started in `{}`: {}.",
            workdir.display(),
            ctx.config
                .roles
                .iter()
                .map(|r| r.display_name.as_str())
                .collect::<Vec<_>>()
                .join("/")
        ),
    )
    .await?;

    tracing::info!(channel_id, workdir = %workdir.display(), "provisioned");
    Ok(())
}

async fn reject_channel(ctx: &Ctx, channel_id: &str, reason: &str) -> anyhow::Result<()> {
    ctx.state.save(
        channel_id,
        &ChannelState {
            status: ChannelStatus::Rejected {
                reason: reason.to_string(),
            },
            workdir: None,
            roles: HashMap::new(),
        },
    )?;
    if let Ok(channel_uuid) = Uuid::parse_str(channel_id) {
        let _ = send_message(
            ctx,
            channel_uuid,
            &format!("⚠️ Rejected workdir setting: {reason}"),
        )
        .await;
    }
    tracing::warn!(channel_id, reason, "rejected");
    Ok(())
}

fn heal_channel(ctx: &Ctx, channel_id: &str, state: &ChannelState) -> anyhow::Result<()> {
    if !matches!(state.status, ChannelStatus::Provisioned) {
        return Ok(());
    }
    let Some(workdir) = &state.workdir else {
        return Ok(());
    };
    let allowlist: Vec<String> = std::iter::once(ctx.client.keys().public_key().to_hex())
        .chain(ctx.config.extra_allowlist.iter().cloned())
        .chain(state.roles.values().map(|r| r.pubkey.clone()))
        .collect();

    let mut updated = state.clone();
    let mut changed = false;
    for role_config in &ctx.config.roles {
        let Some(role_state) = updated.roles.get(&role_config.name) else {
            continue;
        };
        if let Some(pid) = role_state.pid {
            if process_alive(pid) {
                continue;
            }
        }
        tracing::warn!(
            channel_id,
            role = role_config.name,
            "dead agent, restarting"
        );
        let keys = Keys::parse(&role_state.privkey)?;
        let pid = spawn_role(
            ctx,
            channel_id,
            role_config,
            &keys,
            Path::new(workdir),
            &allowlist,
        )?;
        updated.roles.insert(
            role_config.name.clone(),
            RoleState {
                pubkey: role_state.pubkey.clone(),
                privkey: role_state.privkey.clone(),
                pid: Some(pid),
            },
        );
        changed = true;
    }
    if changed {
        ctx.state.save(channel_id, &updated)?;
    }
    Ok(())
}

fn tear_down(ctx: &Ctx, channel_id: &str, state: &ChannelState) -> anyhow::Result<()> {
    for (role, role_state) in &state.roles {
        if let Some(pid) = role_state.pid {
            tracing::info!(
                channel_id,
                role,
                pid,
                "tearing down (channel no longer listed)"
            );
            stop_process(pid);
        }
    }
    let mut updated = state.clone();
    updated.status = ChannelStatus::TornDown;
    ctx.state.save(channel_id, &updated)?;
    Ok(())
}

fn spawn_role(
    ctx: &Ctx,
    channel_id: &str,
    role: &crate::config::RoleConfig,
    keys: &Keys,
    workdir: &Path,
    allowlist: &[String],
) -> anyhow::Result<u32> {
    use std::os::unix::process::CommandExt;

    let log_path = ctx.state.log_path(channel_id, &role.name);
    let log_file = std::fs::File::create(&log_path)?;
    let log_file_err = log_file.try_clone()?;

    let mut cmd = Command::new(&ctx.acp_bin);
    cmd.current_dir(workdir)
        .env("BUZZ_ACP_AGENT_COMMAND", &role.agent_command)
        .env("BUZZ_RELAY_URL", ws_url(&ctx.relay_url_for_admin))
        .env(
            "BUZZ_PRIVATE_KEY",
            keys.secret_key().display_secret().to_string(),
        )
        .arg("--respond-to")
        .arg("allowlist")
        .arg("--respond-to-allowlist")
        .arg(allowlist.join(","))
        .stdin(Stdio::null())
        .stdout(log_file)
        .stderr(log_file_err)
        .process_group(0); // detach from our own process group, like setsid

    if role.subscribe_all {
        cmd.arg("--subscribe").arg("all");
    }
    if let Some(prompt_file) = &role.system_prompt_file {
        let prompt = std::fs::read_to_string(prompt_file)
            .map_err(|e| anyhow::anyhow!("reading system_prompt_file {prompt_file}: {e}"))?;
        cmd.env("BUZZ_ACP_SYSTEM_PROMPT", prompt);
    }

    let child = cmd.spawn()?;
    let pid = child.id();
    // Deliberately not waited on: this is a long-running detached agent
    // process, tracked by pid in state.json, not as a child of ours.
    std::mem::forget(child);
    tracing::info!(channel_id, role = role.name, pid, workdir = %workdir.display(), "launched");
    Ok(pid)
}

fn process_alive(pid: u32) -> bool {
    Path::new(&format!("/proc/{pid}")).exists()
}

/// Sends SIGTERM to a recorded pid. Uses `nix::sys::signal::kill` — a safe
/// wrapper around the POSIX `kill` syscall — so this crate's
/// `#![deny(unsafe_code)]` policy (same as `buzz-acp`) is preserved. A
/// stale/reused pid just gets a harmless signal delivery, or `ESRCH` if it's
/// already gone — either way there's nothing actionable for the caller.
fn stop_process(pid: u32) {
    use nix::sys::signal::{kill, Signal};
    use nix::unistd::Pid;

    let _ = kill(Pid::from_raw(pid as i32), Signal::SIGTERM);
}

fn ws_url(http_url: &str) -> String {
    http_url
        .replacen("http://", "ws://", 1)
        .replacen("https://", "wss://", 1)
}

fn admin_register_member(ctx: &Ctx, pubkey: &str) -> anyhow::Result<()> {
    let status = Command::new(&ctx.admin_bin)
        .arg("add-member")
        .arg("--pubkey")
        .arg(pubkey)
        .arg("--role")
        .arg("member")
        .env("BUZZ_RELAY_PRIVATE_KEY", &ctx.relay_admin_key)
        .env("BUZZ_RELAY_URL", &ctx.relay_url_for_admin)
        .stdout(Stdio::null())
        .status()?;
    if !status.success() {
        anyhow::bail!("buzz-admin add-member failed for {pubkey}: {status}");
    }
    Ok(())
}

async fn add_channel_member(
    ctx: &Ctx,
    channel_id: Uuid,
    pubkey: &str,
    role: &str,
) -> anyhow::Result<()> {
    use buzz_sdk::builders::build_add_member;
    use buzz_sdk::MemberRole;
    let member_role = match role {
        "bot" => MemberRole::Bot,
        _ => MemberRole::Member,
    };
    let builder = build_add_member(channel_id, pubkey, Some(member_role))
        .map_err(|e| anyhow::anyhow!("build_add_member: {e}"))?;
    let event = ctx.client.sign_event(builder)?;
    ctx.client
        .submit_event(event)
        .await
        .map_err(|e| anyhow::anyhow!("add-member submit failed: {e}"))?;
    Ok(())
}

async fn publish_agent_profile(
    ctx: &Ctx,
    keys: &Keys,
    display_name: &str,
    channel_id: &str,
    allowlist: &[String],
) -> anyhow::Result<()> {
    let content = serde_json::json!({
        "name": display_name,
        "agent_type": "agent",
        "channels": [],
        "channel_ids": [channel_id],
        "capabilities": [],
        "status": "online",
        "respond_to": "allowlist",
        "respond_to_allowlist": allowlist,
        // See relay.md #6: "anyone" is deliberate — respond_to_allowlist is
        // the real trigger boundary, and owner_only would need an "owner"
        // field this crate never sets, permanently blocking re-use of a
        // shared identity across channels.
        "channel_add_policy": "anyone",
    })
    .to_string();
    let builder = EventBuilder::new(Kind::Custom(KIND_AGENT_PROFILE as u16), &content).tags([]);
    let event = sign_as(keys, builder)?;
    client_for(ctx, keys)?
        .submit_event(event)
        .await
        .map_err(|e| anyhow::anyhow!("publish-profile failed: {e}"))?;
    Ok(())
}

async fn publish_kind0_profile(ctx: &Ctx, keys: &Keys, display_name: &str) -> anyhow::Result<()> {
    let content =
        serde_json::json!({ "name": display_name, "display_name": display_name }).to_string();
    let builder = EventBuilder::new(Kind::Metadata, &content);
    let event = sign_as(keys, builder)?;
    client_for(ctx, keys)?
        .submit_event(event)
        .await
        .map_err(|e| anyhow::anyhow!("kind:0 profile publish failed: {e}"))?;
    Ok(())
}

/// Relay's `/events` endpoint requires the event's `pubkey` to match the
/// NIP-98-authenticated identity of the request itself, so each agent
/// identity's own events must be submitted through a client authenticated
/// as *that* identity — `ctx.client` (the owner) can't submit on its behalf.
fn client_for(ctx: &Ctx, keys: &Keys) -> anyhow::Result<BuzzClient> {
    BuzzClient::new(ctx.relay_url_for_admin.clone(), keys.clone(), None, None)
        .map_err(|e| anyhow::anyhow!("building per-identity client: {e}"))
}

async fn send_message(ctx: &Ctx, channel_id: Uuid, content: &str) -> anyhow::Result<()> {
    use buzz_sdk::builders::build_message;
    let builder = build_message(channel_id, content, None, &[], false, &[])
        .map_err(|e| anyhow::anyhow!("build_message: {e}"))?;
    let event = ctx.client.sign_event(builder)?;
    ctx.client
        .submit_event(event)
        .await
        .map_err(|e| anyhow::anyhow!("send_message failed: {e}"))?;
    Ok(())
}

fn sign_as(keys: &Keys, builder: EventBuilder) -> anyhow::Result<nostr::Event> {
    builder
        .sign_with_keys(keys)
        .map_err(|e| anyhow::anyhow!("signing failed: {e}"))
}
