//! `buzz teams` — create, list, get, and delete kind:30176 team definitions.
//!
//! A team is a named set of persona definitions plus optional layered
//! instructions. Membership is by persona d-tag, and `create` resolves every
//! `--persona` against the personas this identity has actually published: a team
//! naming a persona that does not exist yet is rejected rather than published as
//! a dangling reference.

use std::collections::{BTreeMap, BTreeSet};

use buzz_core::kind::{KIND_PERSONA, KIND_TEAM};
use buzz_sdk::agent_definitions::{
    build_team_delete, build_team_event, event_d_tag, normalize_d_tag, persona_content_from_event,
    team_content_from_event, TeamEventContent, MAX_D_TAG_LEN,
};

use super::definitions::{
    fetch_head, list_owned, print_event_json, print_write_response, publish_definition,
    read_body_file, read_snapshot, MAX_EVENT_CONTENT_LEN, MAX_TEAM_SNAPSHOT_BYTES,
};
use crate::client::BuzzClient;
use crate::error::CliError;
use crate::validate::sdk_err;
use crate::{TeamCreateArgs, TeamsCmd};

/// Snapshot envelope written by Buzz Desktop's team export.
const TEAM_SNAPSHOT_FORMAT: &str = "buzz-team-snapshot";

pub async fn dispatch(cmd: TeamsCmd, client: &BuzzClient) -> Result<(), CliError> {
    match cmd {
        TeamsCmd::Create(args) => cmd_create(client, args).await,
        TeamsCmd::List { json } => cmd_list(client, json).await,
        TeamsCmd::Get { id, json } => cmd_get(client, &id, json).await,
        TeamsCmd::Delete { id } => cmd_delete(client, &id).await,
    }
}

/// The v1 `team` header fields a team event publishes.
#[derive(serde::Deserialize)]
struct SnapshotTeam {
    name: Option<String>,
    description: Option<String>,
    instructions: Option<String>,
}

/// Resolve `--from` and the individual flags into a team id plus content body.
///
/// `instructions` and `persona_ids` are always `Some`: the outer `None` on the
/// wire means "publisher predates always-publish, preserve local", which a new
/// client must never claim. An empty instruction set is published as `null` and
/// an empty roster as `[]`, so a reader can tell "cleared" from "unknown".
fn resolve_create(args: &TeamCreateArgs) -> Result<(String, TeamEventContent), CliError> {
    let mut name = None;
    let mut description = None;
    let mut instructions = None;
    let mut personas: Vec<String> = Vec::new();
    let mut id_from_file = None;

    if let Some(path) = &args.from {
        let snapshot = read_snapshot(path, TEAM_SNAPSHOT_FORMAT, MAX_TEAM_SNAPSHOT_BYTES)?;
        let raw_team = snapshot
            .get("team")
            .ok_or_else(|| CliError::Usage(format!("'{path}' has no `team` object to publish")))?;
        // Typed, so a wrong-typed field is an error: probing with `as_str` would
        // drop a non-string `instructions` and publish a team missing the text
        // that drives it, reporting success.
        let team: SnapshotTeam = serde_json::from_value(raw_team.clone()).map_err(|e| {
            CliError::Usage(format!(
                "'{path}' team is not a valid v1 team snapshot: {e}"
            ))
        })?;
        name = team.name;
        description = team.description;
        instructions = team.instructions;
        id_from_file = name.clone();

        // Members are embedded agent snapshots, which identify a persona only by
        // display name — there is no id in the export. Keep the name verbatim;
        // cmd_create resolves it against what is actually published, because
        // Desktop publishes personas under their record id (usually a UUID) and
        // slugifying the name here would never match one.
        // Dropping an unreadable member would publish a smaller roster and still
        // report success — silent membership loss is the exact failure the
        // persona_ids tri-state exists to prevent, so refuse instead.
        personas = match snapshot.get("members") {
            None => Vec::new(),
            Some(serde_json::Value::Array(members)) => {
                let mut out = Vec::with_capacity(members.len());
                for (i, m) in members.iter().enumerate() {
                    let name = m
                        .get("definition")
                        .and_then(|d| d.get("name"))
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| {
                            CliError::Usage(format!(
                                "'{path}' member {i} has no string `definition.name`"
                            ))
                        })?;
                    out.push(name.to_owned());
                }
                out
            }
            Some(_) => {
                return Err(CliError::Usage(format!(
                    "'{path}' has a non-array `members` field"
                )))
            }
        };
    }

    if let Some(v) = &args.name {
        name = Some(v.clone());
    }
    if let Some(v) = &args.description {
        description = Some(v.clone());
    }
    match (&args.instructions, &args.instructions_file) {
        (Some(_), Some(_)) => {
            return Err(CliError::Usage(
                "--instructions and --instructions-file are mutually exclusive".into(),
            ))
        }
        (Some(v), None) => instructions = Some(v.clone()),
        (None, Some(path)) => instructions = Some(read_body_file(path, "instructions")?),
        (None, None) => {}
    }
    if !args.persona.is_empty() {
        // Verbatim, like the snapshot path: cmd_create resolves a handle to a
        // published d-tag, accepting either a slug or a persona display name.
        personas = args.persona.clone();
    }

    let name = name.filter(|n| !n.trim().is_empty()).ok_or_else(|| {
        CliError::Usage("--name is required (or supply --from with a team name)".into())
    })?;

    // Duplicate members would publish the same persona twice in one roster.
    let mut seen = BTreeSet::new();
    personas.retain(|p| seen.insert(p.clone()));

    // An explicit --id is used verbatim; only a derived one is slugified. The
    // relay enforces the slug grammar on persona d-tags but only a length bound
    // on team ids, and Buzz Desktop team ids are raw UUIDs or identifiers like
    // `builtin-team:welcome` — normalizing here would address a different
    // coordinate than the one Desktop wrote, making its teams unreachable.
    let id = match &args.id {
        // The relay has no envelope validator for kind:30176 — only a generic
        // length cap — so a blank id would publish at a junk coordinate that
        // later writers collide with, which is last-write-wins data loss. Only
        // an explicit id needs the guard: normalize_d_tag always yields a
        // non-empty slug within MAX_D_TAG_LEN.
        Some(id) => {
            if id.trim().is_empty() {
                return Err(CliError::Usage("--id must not be empty or blank".into()));
            }
            if id.len() > MAX_D_TAG_LEN {
                return Err(CliError::Usage(format!(
                    "--id must not exceed {MAX_D_TAG_LEN} bytes (got {})",
                    id.len()
                )));
            }
            id.clone()
        }
        None => normalize_d_tag(&id_from_file.unwrap_or_else(|| name.clone())),
    };
    let content = TeamEventContent {
        name,
        description,
        instructions: Some(instructions),
        persona_ids: Some(personas),
    };

    // Report the relay's own cap here rather than letting the write fail with an
    // opaque rejection after every member has already been resolved. Only the
    // cap — Desktop applies no text rules to a team, so a CLI-only one would
    // refuse a roster Desktop exported legitimately.
    let len = serde_json::to_string(&content)
        .map(|s| s.len())
        .unwrap_or(0);
    if len > MAX_EVENT_CONTENT_LEN {
        return Err(CliError::Usage(format!(
            "team is too large to publish ({len} bytes, max {MAX_EVENT_CONTENT_LEN}); \
             shorten --instructions"
        )));
    }

    Ok((id, content))
}

/// Resolve each requested member to a published persona d-tag, rejecting any
/// that has no published kind:30175 definition.
///
/// A team event stores d-tags, not definitions, so a missing persona is a
/// dangling reference that surfaces later as an empty seat rather than an error.
///
/// A handle may be a d-tag or a persona's display name: a Desktop team snapshot
/// names its members only by display name, and Desktop publishes personas under
/// their record id — usually a UUID — so matching on the slugified name alone
/// would reject every Desktop-published roster.
async fn resolve_members(
    client: &BuzzClient,
    requested: &[String],
) -> Result<Vec<String>, CliError> {
    if requested.is_empty() {
        return Ok(Vec::new());
    }
    let events = list_owned(client, KIND_PERSONA).await?;
    let mut d_tags: BTreeSet<&str> = BTreeSet::new();
    let mut by_display: BTreeMap<String, Vec<&str>> = BTreeMap::new();
    for event in &events {
        let Some(d_tag) = event_d_tag(event) else {
            continue;
        };
        d_tags.insert(d_tag);
        if let Ok(content) = persona_content_from_event(event) {
            by_display
                .entry(content.display_name.to_lowercase())
                .or_default()
                .push(d_tag);
        }
    }

    match_members(requested, &d_tags, &by_display)
}

/// Match member handles against an index of published personas.
///
/// Tried in order: exact d-tag, unique display name (case-insensitive), then
/// slugified d-tag. Display name outranks the slugified form because a Desktop
/// snapshot names members only by display name — slugifying "Herring" onto an
/// unrelated `herring` d-tag would silently seat the wrong persona. An
/// ambiguous display name is an error rather than a guess, for the same
/// reason.
fn match_members(
    requested: &[String],
    d_tags: &BTreeSet<&str>,
    by_display: &BTreeMap<String, Vec<&str>>,
) -> Result<Vec<String>, CliError> {
    let mut resolved = Vec::with_capacity(requested.len());
    let mut missing = Vec::new();
    for handle in requested {
        if let Some(hit) = d_tags.get(handle.as_str()) {
            resolved.push((*hit).to_owned());
            continue;
        }
        match by_display.get(&handle.to_lowercase()).map(Vec::as_slice) {
            Some([only]) => {
                resolved.push((*only).to_owned());
                continue;
            }
            Some(many) => {
                return Err(CliError::Usage(format!(
                    "'{handle}' matches {} published personas ({}); \
                     pass the slug you want with --persona",
                    many.len(),
                    many.join(", ")
                )));
            }
            _ => {}
        }
        let slug = normalize_d_tag(handle);
        if let Some(hit) = d_tags.get(slug.as_str()) {
            resolved.push((*hit).to_owned());
            continue;
        }
        missing.push(handle.as_str());
    }
    if !missing.is_empty() {
        return Err(CliError::NotFound(format!(
            "no published persona for: {}. Create each one first with \
             `buzz personas create` (see `buzz personas list`).",
            missing.join(", ")
        )));
    }

    // Two handles can resolve to the same persona (a slug and its display name),
    // which would seat it twice.
    let mut seen = BTreeSet::new();
    resolved.retain(|p| seen.insert(p.clone()));
    Ok(resolved)
}

/// Find the published team whose name matches `name`, if exactly one does.
///
/// Ambiguity returns `None` rather than guessing: writing to the wrong
/// coordinate would overwrite an unrelated team.
async fn find_team_by_name(client: &BuzzClient, name: &str) -> Result<Option<String>, CliError> {
    let events = list_owned(client, KIND_TEAM).await?;
    let mut hits: Vec<&str> = Vec::new();
    for event in &events {
        let Some(d_tag) = event_d_tag(event) else {
            continue;
        };
        if let Ok(content) = team_content_from_event(event) {
            if content.name.eq_ignore_ascii_case(name) {
                hits.push(d_tag);
            }
        }
    }
    match hits.as_slice() {
        [only] => Ok(Some((*only).to_owned())),
        _ => Ok(None),
    }
}

async fn cmd_create(client: &BuzzClient, args: TeamCreateArgs) -> Result<(), CliError> {
    let (mut id, mut content) = resolve_create(&args)?;

    // Membership is resolved before the existence check so a typo'd roster fails
    // the same way whether or not the team already exists. resolve_create leaves
    // handles verbatim; the event must carry d-tags.
    let members = resolve_members(client, &content.persona_ids.clone().unwrap_or_default()).await?;
    content.persona_ids = Some(members.clone());

    // A team snapshot carries no id, so a derived one is a slugified name — but
    // Desktop publishes teams under a UUID or an id like `builtin-team:welcome`.
    // Writing the derived id would mint a second coordinate for a team that
    // already exists, leaving Desktop with two records of the same name. Adopt
    // the published team's own id when the name matches, the same way member
    // handles resolve.
    if args.id.is_none() {
        if let Some(existing) = find_team_by_name(client, &content.name).await? {
            if existing != id {
                eprintln!(
                    "note: adopting published team id '{existing}' for '{}'",
                    content.name
                );
                id = existing;
            }
        }
    }

    let head = fetch_head(client, KIND_TEAM, &id).await?;
    if head.is_some() && !args.replace {
        return Err(CliError::Conflict(format!(
            "team '{id}' already exists — pass --replace to overwrite it"
        )));
    }

    let builder = build_team_event(&id, &content).map_err(sdk_err)?;
    let (event, response) = publish_definition(client, builder, head.as_ref()).await?;

    let verb = if head.is_some() {
        "replaced"
    } else {
        "created"
    };
    eprintln!(
        "{verb} team {id} ({}) with {} member(s) — event {}",
        content.name,
        members.len(),
        event.id.to_hex()
    );
    print_write_response(response, "team_id", &id);
    Ok(())
}

async fn cmd_list(client: &BuzzClient, json: bool) -> Result<(), CliError> {
    let events = list_owned(client, KIND_TEAM).await?;
    if json {
        let items: Vec<serde_json::Value> = events
            .iter()
            .map(|e| {
                serde_json::json!({
                    "id": event_d_tag(e),
                    "event_id": e.id.to_hex(),
                    "created_at": e.created_at.as_secs(),
                    "content": team_content_from_event(e).ok(),
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&items)
                .map_err(|e| CliError::Other(format!("failed to render JSON: {e}")))?
        );
        return Ok(());
    }

    if events.is_empty() {
        eprintln!("no teams published by this identity");
        return Ok(());
    }
    for event in &events {
        let id = event_d_tag(event).unwrap_or("<no d-tag>");
        match team_content_from_event(event) {
            Ok(c) => {
                let members = c.persona_ids.unwrap_or_default();
                println!("{id:<24} {:<32} {}", c.name, members.join(", "));
            }
            Err(e) => println!("{id:<24} <unparseable content: {e}>"),
        }
    }
    Ok(())
}

/// Report a missing team, naming the slug when the id would have been one.
///
/// Lookups are verbatim because Desktop writes raw UUIDs and ids like
/// `builtin-team:welcome`, which normalizing would point at a different
/// coordinate. But `create` slugifies an id it derives from `--name`, so
/// `teams get "Red team"` misses the `red-team` it just published.
fn team_not_found(id: &str) -> CliError {
    let slug = normalize_d_tag(id);
    if slug == id {
        return CliError::NotFound(format!("no team '{id}' for this identity"));
    }
    CliError::NotFound(format!(
        "no team '{id}' for this identity — a team created from --name is addressed by its \
         slug, so try '{slug}'"
    ))
}

async fn cmd_get(client: &BuzzClient, id: &str, json: bool) -> Result<(), CliError> {
    // Verbatim: an explicit --id publishes verbatim, so reads must match it.
    let event = fetch_head(client, KIND_TEAM, id)
        .await?
        .ok_or_else(|| team_not_found(id))?;
    if json {
        return print_event_json(&event);
    }

    let c = team_content_from_event(&event).map_err(sdk_err)?;
    println!("id:          {id}");
    println!("name:        {}", c.name);
    println!(
        "description: {}",
        c.description.unwrap_or_else(|| "-".into())
    );
    match c.persona_ids {
        Some(ids) if ids.is_empty() => println!("members:     (none)"),
        Some(ids) => println!("members:     {}", ids.join(", ")),
        // Absent, not empty: this publisher predates always-publish, so its
        // real membership is unknown rather than empty.
        None => println!("members:     (not published — unknown)"),
    }
    println!("event:       {}", event.id.to_hex());
    if let Some(Some(instructions)) = c.instructions {
        println!("\n{instructions}");
    }
    Ok(())
}

async fn cmd_delete(client: &BuzzClient, id: &str) -> Result<(), CliError> {
    // Verbatim: an explicit --id publishes verbatim, so reads must match it.
    let head = fetch_head(client, KIND_TEAM, id).await?;
    if head.is_none() {
        return Err(team_not_found(id));
    }
    let builder = build_team_delete(id, &client.keys().public_key().to_hex()).map_err(sdk_err)?;
    // NIP-09 scopes an `a`-tag delete to versions at or before the tombstone's
    // own created_at, so the tombstone must outrank the head it targets. A team
    // replaced within this same second carries a bumped created_at, and a
    // tombstone stamped with a bare `now` would be silently ignored.
    let (event, response) = publish_definition(client, builder, head.as_ref()).await?;

    // The relay accepts a tombstone that deletes nothing and only debug-logs the
    // miss, so acceptance is not evidence. Re-read the coordinate to confirm.
    //
    // A stale read is not a race: buzz-db documents that "stale deletions can
    // briefly inflate the result set", so a lagging replica can still return the
    // head we just deleted. That head is always STRICTLY older than its own
    // tombstone, which is stamped past it — so a survivor at or past the
    // tombstone's stamp is a different event, one the delete did not cover.
    if let Some(survivor) = fetch_head(client, KIND_TEAM, id).await? {
        if survivor.created_at >= event.created_at {
            return Err(CliError::Conflict(format!(
                "team '{id}' still exists (head at {}); a concurrent write raced the delete",
                survivor.created_at.as_secs()
            )));
        }
    }

    eprintln!("deleted team {id} — tombstone {}", event.id.to_hex());
    print_write_response(response, "team_id", id);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The relay would reject this write anyway, but only after every member has
    /// been resolved and with nothing naming the cause.
    #[test]
    fn a_team_past_the_relay_content_cap_is_refused_locally() {
        let err = resolve_create(&TeamCreateArgs {
            name: Some("Red team".into()),
            instructions: Some("a".repeat(MAX_EVENT_CONTENT_LEN + 1)),
            ..args()
        })
        .unwrap_err();
        assert!(err.to_string().contains("too large to publish"), "{err}");
    }

    /// Desktop applies no text rules to a team, so a CLI-only one would refuse a
    /// roster Desktop exported legitimately. Long-but-publishable text passes.
    #[test]
    fn team_text_is_not_validated_beyond_the_cap() {
        assert!(resolve_create(&TeamCreateArgs {
            name: Some("Red team".into()),
            instructions: Some(format!("Ship\u{200B}it{}", "a".repeat(64 * 1024))),
            ..args()
        })
        .is_ok());
    }

    /// `create` slugifies an id derived from `--name`, so a read with the name
    /// misses. Lookups stay verbatim — Desktop writes ids like
    /// `builtin-team:welcome` — so the error has to name the slug instead.
    #[test]
    fn a_missing_team_names_the_slug_it_would_have() {
        let err = team_not_found("Red team").to_string();
        assert!(err.contains("red-team"), "{err}");
        // An id that is already a slug has nothing to suggest.
        let plain = team_not_found("red-team").to_string();
        assert!(!plain.contains("try"), "{plain}");
    }

    fn args() -> TeamCreateArgs {
        TeamCreateArgs {
            id: None,
            name: None,
            description: None,
            instructions: None,
            instructions_file: None,
            persona: Vec::new(),
            from: None,
            replace: false,
        }
    }

    fn snapshot_file(body: &str) -> (std::path::PathBuf, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "buzz-teams-{}-{}",
            super::super::definitions::now_secs(),
            body.len()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("team.json");
        std::fs::write(&path, body).unwrap();
        (dir, path)
    }

    #[test]
    fn name_is_required() {
        let err = resolve_create(&args()).unwrap_err();
        assert!(err.to_string().contains("--name is required"), "{err}");
    }

    /// A new client must always publish both fields: the outer `None` means
    /// "unknown, preserve local", which would let this write wipe a roster.
    #[test]
    fn always_publishes_instructions_and_members() {
        let (_, content) = resolve_create(&TeamCreateArgs {
            name: Some("Red team".into()),
            ..args()
        })
        .unwrap();
        assert_eq!(content.instructions, Some(None), "cleared, not unknown");
        assert_eq!(content.persona_ids, Some(vec![]), "emptied, not unknown");

        let json = serde_json::to_string(&content).unwrap();
        assert!(json.contains(r#""instructions":null"#), "{json}");
        assert!(json.contains(r#""persona_ids":[]"#), "{json}");
    }

    #[test]
    fn id_defaults_to_the_normalized_name() {
        let (id, _) = resolve_create(&TeamCreateArgs {
            name: Some("Red Team".into()),
            ..args()
        })
        .unwrap();
        assert_eq!(id, "red-team");
    }

    /// Buzz Desktop team ids are raw UUIDs or identifiers like
    /// `builtin-team:welcome`. Slugifying an explicit --id would address a
    /// different coordinate than Desktop wrote, so its teams would be
    /// unreachable from the CLI.
    #[test]
    fn explicit_id_is_used_verbatim() {
        for raw in [
            "builtin-team:welcome",
            "9A1657AC-F7AA-5DB0-B632-D8BBEB6DFB50",
        ] {
            let (id, _) = resolve_create(&TeamCreateArgs {
                id: Some(raw.into()),
                name: Some("Whatever".into()),
                ..args()
            })
            .unwrap();
            assert_eq!(id, raw);
        }
    }

    /// Handles stay verbatim here — `resolve_members` maps them to published
    /// d-tags, because a display name is not necessarily its own slug. Only
    /// exact duplicates collapse at this stage.
    #[test]
    fn members_are_kept_verbatim_and_deduped() {
        let (_, content) = resolve_create(&TeamCreateArgs {
            name: Some("T".into()),
            persona: vec!["Herring".into(), "Herring".into(), "quinby".into()],
            ..args()
        })
        .unwrap();
        assert_eq!(
            content.persona_ids,
            Some(vec!["Herring".into(), "quinby".into()])
        );
    }

    #[test]
    fn instructions_flags_conflict() {
        let err = resolve_create(&TeamCreateArgs {
            name: Some("T".into()),
            instructions: Some("a".into()),
            instructions_file: Some("b".into()),
            ..args()
        })
        .unwrap_err();
        assert!(err.to_string().contains("mutually exclusive"), "{err}");
    }

    /// A team snapshot embeds full agent definitions; the team event references
    /// them by d-tag, so members are reduced to normalized slugs.
    #[test]
    fn from_reads_a_desktop_team_snapshot() {
        let (dir, path) = snapshot_file(
            r#"{
              "format": "buzz-team-snapshot",
              "version": 1,
              "team": {"name": "Red team", "description": "Three skeptics."},
              "members": [
                {"definition": {"name": "Monocle"}},
                {"definition": {"name": "Quinby"}},
                {"definition": {"name": "Herring"}}
              ]
            }"#,
        );
        let (id, content) = resolve_create(&TeamCreateArgs {
            from: Some(path.to_str().unwrap().into()),
            ..args()
        })
        .unwrap();

        assert_eq!(id, "red-team");
        assert_eq!(content.name, "Red team");
        assert_eq!(content.description.as_deref(), Some("Three skeptics."));
        // Display names verbatim: Desktop publishes personas under a UUID d-tag,
        // so these are resolved against the relay, not slugified here.
        assert_eq!(
            content.persona_ids,
            Some(vec!["Monocle".into(), "Quinby".into(), "Herring".into()])
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn persona_flags_replace_the_snapshot_roster() {
        let (dir, path) = snapshot_file(
            r#"{"format":"buzz-team-snapshot","version":1,
                "team":{"name":"Red team"},
                "members":[{"definition":{"name":"Monocle"}}]}"#,
        );
        let (_, content) = resolve_create(&TeamCreateArgs {
            from: Some(path.to_str().unwrap().into()),
            persona: vec!["herring".into()],
            ..args()
        })
        .unwrap();
        assert_eq!(content.persona_ids, Some(vec!["herring".into()]));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Silently dropping a member would publish a smaller roster and still
    /// report success — the exact loss the `persona_ids` tri-state guards.
    #[test]
    fn from_rejects_a_member_without_a_definition_name() {
        let (dir, path) = snapshot_file(
            r#"{"format":"buzz-team-snapshot","version":1,
                "team":{"name":"Red team"},
                "members":[{"definition":{"name":"Monocle"}},{"definition":{}}]}"#,
        );
        let err = resolve_create(&TeamCreateArgs {
            from: Some(path.to_str().unwrap().into()),
            ..args()
        })
        .unwrap_err();
        assert!(err.to_string().contains("member 1 has no string"), "{err}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn from_rejects_a_non_array_members_field() {
        let (dir, path) = snapshot_file(
            r#"{"format":"buzz-team-snapshot","version":1,
                "team":{"name":"Red team"},"members":{"a":1}}"#,
        );
        let err = resolve_create(&TeamCreateArgs {
            from: Some(path.to_str().unwrap().into()),
            ..args()
        })
        .unwrap_err();
        assert!(err.to_string().contains("non-array `members`"), "{err}");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Probing with `as_str` would drop a wrong-typed field and publish a team
    /// missing the instructions that drive it, reporting success.
    #[test]
    fn from_rejects_a_wrong_typed_team_field() {
        let (dir, path) = snapshot_file(
            r#"{"format":"buzz-team-snapshot","version":1,
                "team":{"name":"Red team","instructions":["be terse"]}}"#,
        );
        let err = resolve_create(&TeamCreateArgs {
            from: Some(path.to_str().unwrap().into()),
            ..args()
        })
        .unwrap_err();
        assert!(
            err.to_string().contains("not a valid v1 team snapshot"),
            "{err}"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// The relay only length-bounds a team d-tag, so the CLI is the sole guard
    /// against publishing at a junk coordinate.
    #[test]
    fn blank_explicit_id_is_refused() {
        let err = resolve_create(&TeamCreateArgs {
            id: Some("   ".into()),
            name: Some("Red team".into()),
            ..args()
        })
        .unwrap_err();
        assert!(err.to_string().contains("must not be empty"), "{err}");
    }

    #[test]
    fn oversized_explicit_id_is_refused() {
        let err = resolve_create(&TeamCreateArgs {
            id: Some("t".repeat(MAX_D_TAG_LEN + 1)),
            name: Some("Red team".into()),
            ..args()
        })
        .unwrap_err();
        assert!(err.to_string().contains("must not exceed"), "{err}");
    }

    /// The derived path carries no id guard because normalization can't
    /// produce a blank or oversized d-tag — pin that so the omission stays true.
    #[test]
    fn derived_id_is_always_within_the_relay_bound() {
        // A blank name never gets this far — --name rejects it first.
        for name in ["!!!", "-", &"z".repeat(MAX_D_TAG_LEN * 2)] {
            let (id, _) = resolve_create(&TeamCreateArgs {
                name: Some(name.into()),
                ..args()
            })
            .unwrap();
            assert!(!id.trim().is_empty(), "blank id from {name:?}");
            assert!(id.len() <= MAX_D_TAG_LEN, "oversized id from {name:?}");
        }
    }

    #[test]
    fn from_rejects_an_agent_snapshot() {
        let (dir, path) = snapshot_file(r#"{"format":"buzz-agent-snapshot","version":1}"#);
        let err = resolve_create(&TeamCreateArgs {
            from: Some(path.to_str().unwrap().into()),
            ..args()
        })
        .unwrap_err();
        assert!(
            err.to_string().contains("expected 'buzz-team-snapshot'"),
            "{err}"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    // ── member resolution ────────────────────────────────────────────────────

    fn index<'a>(
        published: &[(&'a str, &'a str)],
    ) -> (BTreeSet<&'a str>, BTreeMap<String, Vec<&'a str>>) {
        let mut d_tags = BTreeSet::new();
        let mut by_display: BTreeMap<String, Vec<&str>> = BTreeMap::new();
        for (d_tag, display) in published {
            d_tags.insert(*d_tag);
            by_display
                .entry(display.to_lowercase())
                .or_default()
                .push(*d_tag);
        }
        (d_tags, by_display)
    }

    /// The Desktop→CLI case: personas published under UUID d-tags, a team
    /// snapshot naming them only by display name.
    #[test]
    fn display_names_resolve_to_uuid_d_tags() {
        let (d, n) = index(&[
            ("6f1e2c9a-0b3d-4e5f-8a7b-1c2d3e4f5a6b", "Monocle"),
            ("7a2f3d0b-1c4e-5f6a-9b8c-2d3e4f5a6b7c", "Herring"),
        ]);
        let got = match_members(&["Monocle".into(), "Herring".into()], &d, &n).unwrap();
        assert_eq!(
            got,
            vec![
                "6f1e2c9a-0b3d-4e5f-8a7b-1c2d3e4f5a6b".to_string(),
                "7a2f3d0b-1c4e-5f6a-9b8c-2d3e4f5a6b7c".to_string(),
            ]
        );
    }

    /// The CLI-only case still works: the slug is the d-tag.
    #[test]
    fn slugs_resolve_directly() {
        let (d, n) = index(&[("herring", "Herring")]);
        assert_eq!(
            match_members(&["herring".into()], &d, &n).unwrap(),
            vec!["herring".to_string()]
        );
        // A display name that slugifies onto its own d-tag resolves too.
        assert_eq!(
            match_members(&["Herring".into()], &d, &n).unwrap(),
            vec!["herring".to_string()]
        );
    }

    /// A d-tag hit must win over a display-name hit pointing elsewhere.
    #[test]
    fn exact_d_tag_wins_over_display_name() {
        let (d, n) = index(&[("herring", "Quinby"), ("quinby-2", "Herring")]);
        assert_eq!(
            match_members(&["herring".into()], &d, &n).unwrap(),
            vec!["herring".to_string()]
        );
    }

    #[test]
    fn ambiguous_display_name_is_refused() {
        let (d, n) = index(&[("uuid-a", "Herring"), ("uuid-b", "Herring")]);
        let err = match_members(&["Herring".into()], &d, &n).unwrap_err();
        assert!(
            err.to_string().contains("matches 2 published personas"),
            "{err}"
        );
    }

    /// A Desktop snapshot names members by display name, so a unique display
    /// name must outrank a slugified d-tag collision: binding "Herring" to an
    /// unrelated `herring` slug seats the wrong persona and reports success.
    #[test]
    fn unique_display_name_wins_over_a_slugified_collision() {
        let (d, n) = index(&[("herring", "Old Herring"), ("uuid-h", "Herring")]);
        assert_eq!(
            match_members(&["Herring".into()], &d, &n).unwrap(),
            vec!["uuid-h".to_string()]
        );
    }

    /// When the display name is ambiguous, the slugified collision must not
    /// short-circuit the ambiguity error into a silent wrong seat.
    #[test]
    fn ambiguity_beats_a_slugified_collision() {
        let (d, n) = index(&[("herring", "Herring"), ("uuid-h", "Herring")]);
        let err = match_members(&["Herring".into()], &d, &n).unwrap_err();
        assert!(
            err.to_string().contains("matches 2 published personas"),
            "{err}"
        );
    }

    /// The slugified form still resolves when no display name matches — the
    /// CLI-only path where the handle is a prettied form of the slug.
    #[test]
    fn slugified_d_tag_resolves_when_display_name_misses() {
        let (d, n) = index(&[("code-reviewer", "The Reviewer")]);
        assert_eq!(
            match_members(&["Code Reviewer".into()], &d, &n).unwrap(),
            vec!["code-reviewer".to_string()]
        );
    }

    #[test]
    fn unpublished_member_is_refused() {
        let (d, n) = index(&[("herring", "Herring")]);
        let err = match_members(&["ghost".into()], &d, &n).unwrap_err();
        assert!(
            err.to_string().contains("no published persona for: ghost"),
            "{err}"
        );
    }

    /// A slug and its display name are one persona, not two seats.
    #[test]
    fn handles_converging_on_one_persona_dedupe() {
        let (d, n) = index(&[("herring", "Herring")]);
        assert_eq!(
            match_members(&["herring".into(), "Herring".into()], &d, &n).unwrap(),
            vec!["herring".to_string()]
        );
    }
}
