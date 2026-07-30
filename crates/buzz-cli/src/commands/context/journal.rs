use std::ffi::OsString;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use nostr::Kind;

use super::profile::{CredentialProvider, ProfileEnvironment, ResolvedProfile};
use super::runtime::{event_builder, hostname, read_text, tag, ContextRuntime};
use crate::commands::mem;
use crate::error::CliError;
use crate::MemCmd;

pub async fn save(
    profile: &ResolvedProfile,
    environment: &ProfileEnvironment,
    scope: &str,
    file: Option<&Path>,
) -> Result<(), CliError> {
    validate_scope(scope)?;
    let runtime = ContextRuntime::new(profile, environment)?;
    let client = runtime.local_journal_client()?;
    let owner = owner_pubkey(&client);
    let value = read_text(file)?;
    mem::dispatch(
        MemCmd::Set {
            slug: format!("mem/ctx/{scope}"),
            value,
            owner: Some(owner),
            allow_empty: false,
        },
        &client,
    )
    .await
}

pub async fn load(
    profile: &ResolvedProfile,
    environment: &ProfileEnvironment,
    scope: &str,
) -> Result<(), CliError> {
    validate_scope(scope)?;
    let runtime = ContextRuntime::new(profile, environment)?;
    let local = runtime.local_journal_client()?;
    let local_owner = owner_pubkey(&local);
    let command = || MemCmd::Get {
        slug: format!("mem/ctx/{scope}"),
        owner: Some(local_owner.clone()),
        agent: None,
    };
    match mem::dispatch(command(), &local).await {
        Ok(()) => Ok(()),
        Err(local_error) => {
            let cloud = runtime.cloud_journal_client().map_err(|_| local_error)?;
            mem::dispatch(
                MemCmd::Get {
                    slug: format!("mem/ctx/{scope}"),
                    owner: Some(local_owner),
                    agent: None,
                },
                &cloud,
            )
            .await
        }
    }
}

pub async fn list(
    profile: &ResolvedProfile,
    environment: &ProfileEnvironment,
    json: bool,
) -> Result<(), CliError> {
    let runtime = ContextRuntime::new(profile, environment)?;
    let local = runtime.local_journal_client()?;
    let owner = owner_pubkey(&local);
    let command = || MemCmd::Ls {
        owner: Some(owner.clone()),
        agent: None,
        json,
    };
    match mem::dispatch(command(), &local).await {
        Ok(()) => Ok(()),
        Err(local_error) => {
            let cloud = runtime.cloud_journal_client().map_err(|_| local_error)?;
            mem::dispatch(
                MemCmd::Ls {
                    owner: Some(owner),
                    agent: None,
                    json,
                },
                &cloud,
            )
            .await
        }
    }
}

pub async fn log(
    profile: &ResolvedProfile,
    environment: &ProfileEnvironment,
    project: &str,
    message: &str,
    context: Option<&str>,
) -> Result<String, CliError> {
    validate_label(project, "project")?;
    if message.trim().is_empty() {
        return Err(CliError::Usage("session message must not be empty".into()));
    }
    let runtime = ContextRuntime::new(profile, environment)?;
    let (_, role) = runtime.local_event_client()?;
    let identity = runtime.identity_label(role);
    let machine = hostname();
    let session = format!("session:{project}");
    let context = session_context(profile, context)?;
    let builder = event_builder(
        message,
        session_tags(&session, &identity, &machine, context)?,
        None,
    );
    runtime.post_builder(builder).await
}

pub async fn sessions(
    profile: &ResolvedProfile,
    environment: &ProfileEnvironment,
    project: &str,
    limit: u32,
    json: bool,
) -> Result<(), CliError> {
    validate_label(project, "project")?;
    if limit == 0 || limit > 500 {
        return Err(CliError::Usage(
            "session limit must be between 1 and 500".into(),
        ));
    }
    let runtime = ContextRuntime::new(profile, environment)?;
    let session = format!("session:{project}");
    let events = runtime
        .query_preferred(&[serde_json::json!({
            "kinds": [Kind::TextNote.as_u16()],
            "#t": [session],
            "limit": limit,
        })])
        .await?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&events).map_err(|error| CliError::Other(format!(
                "session serialization failed: {error}"
            )))?
        );
        return Ok(());
    }
    for event in events {
        let agent = event_tag(&event, "agent").unwrap_or("?");
        let machine = event_tag(&event, "machine").unwrap_or("?");
        let attested = event
            .tags
            .iter()
            .any(|tag| tag.as_slice().first().map(String::as_str) == Some("auth"));
        let timestamp = DateTime::<Utc>::from_timestamp(event.created_at.as_secs() as i64, 0)
            .map(|value| value.to_rfc3339())
            .unwrap_or_else(|| event.created_at.as_secs().to_string());
        println!(
            "{timestamp}  [{agent}@{machine}{}]  {}",
            if attested { " ✓" } else { "" },
            event.content
        );
    }
    Ok(())
}

pub async fn share(
    profile: &ResolvedProfile,
    environment: &ProfileEnvironment,
    file: Option<&Path>,
    message: Option<&str>,
) -> Result<String, CliError> {
    let content = match message {
        Some(message) if !message.trim().is_empty() => message.to_string(),
        Some(_) => return Err(CliError::Usage("share message must not be empty".into())),
        None => read_text(file)?,
    };
    if content.trim().is_empty() {
        return Err(CliError::Usage("share content must not be empty".into()));
    }
    let runtime = ContextRuntime::new(profile, environment)?;
    let (_, role) = runtime.local_event_client()?;
    let identity = runtime.identity_label(role);
    let machine = hostname();
    let mut tags = vec![
        tag(&["t", "shared:tooling"])?,
        tag(&["agent", &identity])?,
        tag(&["machine", &machine])?,
    ];
    if let Ok(context) = runtime.default_context() {
        tags.push(tag(&["h", context])?);
    }
    runtime
        .post_builder(event_builder(content, tags, None))
        .await
}

pub fn sync(
    profile: &ResolvedProfile,
    environment: &ProfileEnvironment,
    dry_run: bool,
) -> Result<(), CliError> {
    let invocation = replication_invocation(profile)?;
    if dry_run {
        println!("replication dry run");
        println!("  executable  {}", invocation.executable.display());
        println!("  journal     {}", invocation.journal.display());
        println!("  rendezvous  {}", invocation.rendezvous);
        println!("  source      {}", invocation.source);
        println!("  credential  {}", invocation.credential);
        println!("  cursor      {}", invocation.cursor_file.display());
        println!(
            "  selection   {}",
            invocation
                .streams_file
                .as_deref()
                .map_or_else(|| "whole journal".into(), |path| path.display().to_string())
        );
        return Ok(());
    }
    let path = environment
        .variables
        .get("PATH")
        .map(String::as_str)
        .unwrap_or("/usr/bin:/bin:/usr/sbin:/sbin");
    let status = std::process::Command::new(&invocation.executable)
        .args(replication_arguments(&invocation))
        .env_clear()
        .env("PATH", path)
        .env("LANG", "C.UTF-8")
        .status()
        .map_err(|error| {
            CliError::Other(format!(
                "could not start replication runtime {}: {error}",
                invocation.executable.display()
            ))
        })?;
    if !status.success() {
        return Err(CliError::Other(format!(
            "replication runtime exited with {status}"
        )));
    }
    Ok(())
}

struct ReplicationInvocation {
    executable: PathBuf,
    journal: PathBuf,
    rendezvous: String,
    source: String,
    credential: String,
    cursor_file: PathBuf,
    streams_file: Option<PathBuf>,
}

fn replication_invocation(profile: &ResolvedProfile) -> Result<ReplicationInvocation, CliError> {
    profile.require_ready()?;
    let rendezvous = profile.file.relays.rendezvous.as_deref().ok_or_else(|| {
        CliError::Usage(format!(
            "profile {} does not configure relays.rendezvous",
            profile.name
        ))
    })?;
    let executable = profile.file.runtime.relay_push.as_deref().ok_or_else(|| {
        CliError::Usage(format!(
            "profile {} does not configure runtime.relay_push",
            profile.name
        ))
    })?;
    if !executable.is_file() {
        return Err(CliError::NotFound(format!(
            "configured relay-push runtime is absent: {}",
            executable.display()
        )));
    }
    let transport = profile
        .identity("replication_transport")
        .ok_or_else(|| CliError::Auth("replication_transport role is not configured".into()))?;
    if transport.provider != CredentialProvider::File {
        return Err(CliError::Usage(
            "the configured relay-push runtime currently requires a file credential provider"
                .into(),
        ));
    }
    let source = profile
        .file
        .replication
        .source
        .as_deref()
        .ok_or_else(|| CliError::Usage("replication.source is not configured".into()))?;
    let cursor_file = profile
        .file
        .replication
        .cursor_file
        .clone()
        .ok_or_else(|| {
            CliError::Usage(format!(
                "profile {} does not configure replication.cursor_file",
                profile.name
            ))
        })?;
    let streams_file = profile.file.replication.streams_file.clone();
    if let Some(streams_file) = streams_file.as_deref() {
        if !streams_file.is_file() {
            return Err(CliError::NotFound(format!(
                "configured replication stream selection is absent: {}",
                streams_file.display()
            )));
        }
    } else if !profile.file.replication.streams.is_empty() {
        return Err(CliError::Usage(format!(
            "profile {} lists replication.streams but does not configure replication.streams_file",
            profile.name
        )));
    }
    Ok(ReplicationInvocation {
        executable: executable.to_path_buf(),
        journal: profile.journal.clone(),
        rendezvous: rendezvous.to_string(),
        source: source.to_string(),
        credential: transport.reference.clone(),
        cursor_file,
        streams_file,
    })
}

fn replication_arguments(invocation: &ReplicationInvocation) -> Vec<OsString> {
    let mut arguments = vec![
        "--data".into(),
        invocation.journal.clone().into_os_string(),
        "--to".into(),
        invocation.rendezvous.clone().into(),
        "--source".into(),
        invocation.source.clone().into(),
        "--key".into(),
        invocation.credential.clone().into(),
        "--cursor-file".into(),
        invocation.cursor_file.clone().into_os_string(),
    ];
    if let Some(streams_file) = &invocation.streams_file {
        arguments.push("--streams".into());
        arguments.push(streams_file.clone().into_os_string());
    }
    arguments
}

fn owner_pubkey(client: &crate::client::BuzzClient) -> String {
    client
        .auth_tag_owner_hex()
        .unwrap_or_else(|| client.keys().public_key().to_hex())
}

fn session_context<'a>(
    profile: &'a ResolvedProfile,
    explicit: Option<&'a str>,
) -> Result<&'a str, CliError> {
    select_session_context(
        &profile.name,
        profile.file.context.default_h.as_deref(),
        explicit,
    )
}

fn select_session_context<'a>(
    profile_name: &str,
    configured: Option<&'a str>,
    explicit: Option<&'a str>,
) -> Result<&'a str, CliError> {
    let context = explicit.or(configured).ok_or_else(|| {
        CliError::Usage(format!(
            "profile {profile_name} does not configure context.default_h; pass --context"
        ))
    })?;
    if context.trim().is_empty() {
        return Err(CliError::Usage("session context must not be empty".into()));
    }
    Ok(context)
}

fn session_tags(
    session: &str,
    identity: &str,
    machine: &str,
    context: &str,
) -> Result<Vec<nostr::Tag>, CliError> {
    Ok(vec![
        tag(&["t", session])?,
        tag(&["h", context])?,
        tag(&["agent", identity])?,
        tag(&["machine", machine])?,
    ])
}

fn validate_scope(scope: &str) -> Result<(), CliError> {
    if scope.is_empty()
        || scope.split('/').any(|segment| {
            segment.is_empty()
                || !segment.bytes().all(|byte| {
                    byte.is_ascii_lowercase()
                        || byte.is_ascii_digit()
                        || matches!(byte, b'-' | b'_')
                })
        })
    {
        return Err(CliError::Usage(
            "context scopes must be lowercase slug segments separated by '/'".into(),
        ));
    }
    Ok(())
}

fn validate_label(value: &str, label: &str) -> Result<(), CliError> {
    if value.is_empty()
        || !value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_' | b'.')
        })
    {
        return Err(CliError::Usage(format!(
            "{label} must contain lowercase letters, digits, '-', '_' or '.'"
        )));
    }
    Ok(())
}

fn event_tag<'a>(event: &'a nostr::Event, name: &str) -> Option<&'a str> {
    event.tags.iter().find_map(|tag| {
        let values = tag.as_slice();
        (values.first().map(String::as_str) == Some(name))
            .then(|| values.get(1).map(String::as_str))
            .flatten()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_scope_is_path_safe() {
        assert!(validate_scope("buzz/portable-relay").is_ok());
        for invalid in [
            "Buzz/context",
            "../context",
            "buzz//context",
            "buzz/context.md",
        ] {
            assert!(validate_scope(invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn replication_arguments_preserve_cursor_and_stream_selection() {
        let invocation = ReplicationInvocation {
            executable: PathBuf::from("/opt/buzz/buzz-relay-push"),
            journal: PathBuf::from("/state/sovereign.ndjson"),
            rendezvous: "https://relay.example".into(),
            source: "node/private".into(),
            credential: "/run/credentials/transport.key".into(),
            cursor_file: PathBuf::from("/state/cursors/private.push-cursor"),
            streams_file: Some(PathBuf::from("/state/streams.json")),
        };
        let arguments = replication_arguments(&invocation);
        assert!(arguments
            .windows(2)
            .any(|pair| pair == ["--cursor-file", "/state/cursors/private.push-cursor"]));
        assert!(arguments
            .windows(2)
            .any(|pair| pair == ["--streams", "/state/streams.json"]));
    }

    #[test]
    fn session_residue_carries_its_bounded_context() {
        let tags = session_tags("session:buzz", "claude-code", "host-a", "shared/evolution")
            .expect("tags");
        assert!(tags.iter().any(|tag| {
            tag.as_slice() == [String::from("h"), String::from("shared/evolution")]
        }));
    }

    #[test]
    fn session_context_defaults_and_can_be_overridden() {
        assert_eq!(
            select_session_context("solo", Some("configured"), None).expect("default"),
            "configured"
        );
        assert_eq!(
            select_session_context("solo", Some("configured"), Some("explicit")).expect("override"),
            "explicit"
        );
        assert!(select_session_context("solo", None, None).is_err());
    }
}
