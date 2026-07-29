use std::collections::{BTreeMap, BTreeSet};

use nostr::{Event, EventBuilder, Kind, PublicKey};
use serde_json::Value;

use super::profile::{ProfileEnvironment, ResolvedProfile};
use super::runtime::{tag, ContextRuntime};
use crate::error::CliError;

const AGREEMENT_KIND: u16 = 30_700;

pub async fn export(
    profile: &ResolvedProfile,
    environment: &ProfileEnvironment,
    stream: &str,
    readers: &[String],
) -> Result<String, CliError> {
    validate_stream(stream)?;
    validate_pubkeys(readers, "reader")?;
    let streams_path = profile.data_root.join("streams.json");
    let streams: Value =
        serde_json::from_slice(&std::fs::read(&streams_path).map_err(|error| {
            CliError::Usage(format!(
                "could not read {}: {error}",
                streams_path.display()
            ))
        })?)
        .map_err(|error| {
            CliError::Usage(format!(
                "stream catalog {} is invalid JSON: {error}",
                streams_path.display()
            ))
        })?;
    let selection = streams.get(stream).cloned().ok_or_else(|| {
        CliError::NotFound(format!(
            "stream {stream} is not declared in {}",
            streams_path.display()
        ))
    })?;
    let content = serde_json::json!({
        "status": "active",
        "selection": selection,
        "artifacts": "referenced",
    });
    post_declaration(
        profile,
        environment,
        &format!("export/{stream}"),
        readers,
        Vec::new(),
        content,
    )
    .await
}

pub async fn admit(
    profile: &ResolvedProfile,
    environment: &ProfileEnvironment,
    source: &str,
    principal: &str,
    verification_keys: &[String],
) -> Result<String, CliError> {
    validate_stream(source)?;
    validate_principal(principal)?;
    validate_pubkeys(verification_keys, "verification")?;
    let pin = pin_for_export(profile, environment, source).await?;
    let event_id = post_declaration(
        profile,
        environment,
        &format!("admit/{source}"),
        verification_keys,
        pin.iter()
            .map(|pin| tag(&["e", pin]))
            .collect::<Result<Vec<_>, _>>()?,
        serde_json::json!({
            "status": "active",
            "principal": principal,
            "retention": {"keep": "journal"},
        }),
    )
    .await?;
    if let Some(pin) = pin {
        println!("pinned export: {pin}");
    } else {
        println!("warning: no export head found; re-admit after the export exists");
    }
    Ok(event_id)
}

pub async fn read(
    profile: &ResolvedProfile,
    environment: &ProfileEnvironment,
    stream: &str,
    principal: &str,
    reader_keys: &[String],
) -> Result<String, CliError> {
    validate_stream(stream)?;
    validate_principal(principal)?;
    validate_pubkeys(reader_keys, "reader")?;
    let pin = pin_for_export(profile, environment, stream).await?;
    let event_id = post_declaration(
        profile,
        environment,
        &format!("read/{stream}"),
        reader_keys,
        pin.iter()
            .map(|pin| tag(&["e", pin]))
            .collect::<Result<Vec<_>, _>>()?,
        serde_json::json!({
            "status": "active",
            "principal": principal,
        }),
    )
    .await?;
    if let Some(pin) = pin {
        println!("pinned export: {pin}");
    } else {
        println!("warning: no export head found; grant is unpinned");
    }
    Ok(event_id)
}

pub async fn steward(
    profile: &ResolvedProfile,
    environment: &ProfileEnvironment,
    principal: &str,
    steward_pubkey: &str,
) -> Result<String, CliError> {
    validate_principal(principal)?;
    validate_pubkeys(&[steward_pubkey.to_string()], "steward")?;
    let runtime = ContextRuntime::new(profile, environment)?;
    let node = runtime.node_label();
    post_declaration(
        profile,
        environment,
        &format!("steward/{node}"),
        &[steward_pubkey.to_string()],
        vec![tag(&["h", "shared/steward-reports"])?],
        serde_json::json!({
            "status": "active",
            "principal": principal,
            "powers": ["observe", "report"],
        }),
    )
    .await
}

pub async fn match_stream(
    profile: &ResolvedProfile,
    environment: &ProfileEnvironment,
    stream: &str,
) -> Result<(), CliError> {
    validate_stream(stream)?;
    let runtime = ContextRuntime::new(profile, environment)?;
    let events = runtime
        .query_preferred(&[serde_json::json!({
            "kinds": [AGREEMENT_KIND],
            "#d": [format!("export/{stream}"), format!("admit/{stream}")],
            "limit": 100,
        })])
        .await?;
    let export = newest_head(&events, &format!("export/{stream}"));
    let admit = newest_head(&events, &format!("admit/{stream}"));
    let message = match (export, admit) {
        (None, _) => format!("UNMATCHED: no export declaration for {stream}"),
        (Some(export), None) => format!(
            "OFFER OPEN: export by {} (event {}), no admit half yet",
            export.pubkey.to_hex(),
            export.id.to_hex()
        ),
        (Some(export), Some(admit)) => {
            let export_status = status(export);
            let admit_status = status(admit);
            let pin = first_tag(admit, "e");
            if export_status != "active" {
                format!("UNMATCHED: export status {export_status}")
            } else if admit_status != "active" {
                format!("UNMATCHED: admit status {admit_status}")
            } else if pin.is_none() {
                format!(
                    "UNPINNED: admit {} carries no export pin; re-admit against {}",
                    admit.id.to_hex(),
                    export.id.to_hex()
                )
            } else if pin != Some(export.id.to_hex()) {
                format!(
                    "DRIFT: admit pins {} but export head is {}; re-pin required",
                    pin.as_deref().unwrap_or("absent"),
                    export.id.to_hex()
                )
            } else if !tag_values(export, "p").contains(&admit.pubkey.to_hex()) {
                format!(
                    "UNMATCHED: admit author {} is not an offered party on the export",
                    admit.pubkey.to_hex()
                )
            } else {
                format!(
                    "MATCHED: {stream} — export {} ({}) ⇄ admit {} ({})",
                    export.id.to_hex(),
                    export.pubkey.to_hex(),
                    admit.id.to_hex(),
                    admit.pubkey.to_hex()
                )
            }
        }
    };
    println!("{message}");
    Ok(())
}

pub async fn list(
    profile: &ResolvedProfile,
    environment: &ProfileEnvironment,
    json: bool,
) -> Result<(), CliError> {
    let runtime = ContextRuntime::new(profile, environment)?;
    let events = runtime
        .query_preferred(&[serde_json::json!({
            "kinds": [AGREEMENT_KIND],
            "limit": 1000,
        })])
        .await?;
    let heads = effective_heads(&events);
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&heads.into_values().collect::<Vec<_>>())
                .map_err(|error| CliError::Other(format!("agreement JSON failed: {error}")))?
        );
    } else {
        for (coordinate, event) in heads {
            println!(
                "{coordinate}  {}  by {}  event {}",
                status(event),
                event.pubkey.to_hex(),
                event.id.to_hex()
            );
        }
    }
    Ok(())
}

pub async fn status_view(
    profile: &ResolvedProfile,
    environment: &ProfileEnvironment,
    only_node: Option<&str>,
    json: bool,
) -> Result<(), CliError> {
    let runtime = ContextRuntime::new(profile, environment)?;
    let owner = runtime.local_journal_client()?.keys().public_key().to_hex();
    let events = runtime
        .query_preferred(&[serde_json::json!({
            "kinds": [AGREEMENT_KIND],
            "limit": 1000,
        })])
        .await?;
    let heads = effective_heads(&events);
    let mine = heads
        .values()
        .filter(|event| event.pubkey.to_hex() == owner)
        .copied()
        .collect::<Vec<_>>();
    if json {
        let nodes = governed_nodes(&mine, only_node);
        let result = nodes
            .iter()
            .map(|node| {
                let declarations = mine
                    .iter()
                    .filter(|event| first_tag(event, "n").as_deref() == Some(node.as_str()))
                    .map(|event| {
                        serde_json::json!({
                            "coordinate": first_tag(event, "d"),
                            "status": status(event),
                            "event_id": event.id.to_hex(),
                            "principals": tag_values(event, "p"),
                        })
                    })
                    .collect::<Vec<_>>();
                serde_json::json!({"node": node, "declarations": declarations})
            })
            .collect::<Vec<_>>();
        println!(
            "{}",
            serde_json::to_string_pretty(&result)
                .map_err(|error| CliError::Other(format!("status JSON failed: {error}")))?
        );
        return Ok(());
    }
    for node in governed_nodes(&mine, only_node) {
        println!("■ node {node}");
        for role in ["admit", "read", "export"] {
            let matching = mine
                .iter()
                .filter(|event| {
                    first_tag(event, "n").as_deref() == Some(node.as_str())
                        && first_tag(event, "d")
                            .is_some_and(|coordinate| coordinate.starts_with(&format!("{role}/")))
                })
                .collect::<Vec<_>>();
            if matching.is_empty() {
                println!("  {role}: unclaimed — bootstrap file/env governs");
            } else {
                let active = matching
                    .iter()
                    .filter(|event| status(event) == "active")
                    .count();
                println!(
                    "  {role}: journal-governed ({active} active / {} heads)",
                    matching.len()
                );
                for event in matching {
                    println!(
                        "    {}  {} key(s)",
                        first_tag(event, "d").unwrap_or_else(|| "?".into()),
                        tag_values(event, "p").len()
                    );
                }
            }
        }
        println!();
    }
    let foreign = heads
        .values()
        .filter(|event| event.pubkey.to_hex() != owner)
        .collect::<Vec<_>>();
    if !foreign.is_empty() {
        println!("■ foreign halves (relationship state, never config)");
        for event in foreign {
            println!(
                "  {}  {}  by {}",
                first_tag(event, "d").unwrap_or_else(|| "?".into()),
                status(event),
                event.pubkey.to_hex()
            );
        }
    }
    Ok(())
}

async fn post_declaration(
    profile: &ResolvedProfile,
    environment: &ProfileEnvironment,
    coordinate: &str,
    principals: &[String],
    extra_tags: Vec<nostr::Tag>,
    content: Value,
) -> Result<String, CliError> {
    let runtime = ContextRuntime::new(profile, environment)?;
    let node = runtime.node_label();
    let mut tags = vec![tag(&["d", coordinate])?, tag(&["n", node])?];
    tags.extend(
        principals
            .iter()
            .map(|principal| tag(&["p", principal]))
            .collect::<Result<Vec<_>, _>>()?,
    );
    tags.extend(extra_tags);
    let builder = EventBuilder::new(
        Kind::Custom(AGREEMENT_KIND),
        serde_json::to_string(&content)
            .map_err(|error| CliError::Other(format!("agreement JSON failed: {error}")))?,
    )
    .tags(tags);
    let event_id = runtime.post_owner_builder(builder).await?;
    println!("declared {coordinate} on node {node}: {event_id}");
    Ok(event_id)
}

async fn pin_for_export(
    profile: &ResolvedProfile,
    environment: &ProfileEnvironment,
    stream: &str,
) -> Result<Option<String>, CliError> {
    let runtime = ContextRuntime::new(profile, environment)?;
    let events = runtime
        .query_preferred(&[serde_json::json!({
            "kinds": [AGREEMENT_KIND],
            "#d": [format!("export/{stream}")],
            "limit": 100,
        })])
        .await?;
    Ok(newest_head(&events, &format!("export/{stream}")).map(|event| event.id.to_hex()))
}

fn effective_heads(events: &[Event]) -> BTreeMap<String, &Event> {
    let mut heads = BTreeMap::new();
    for event in events {
        let Some(coordinate) = first_tag(event, "d") else {
            continue;
        };
        let key = format!("{}:{coordinate}", event.pubkey.to_hex());
        let replace = heads.get(&key).is_none_or(|current: &&Event| {
            (event.created_at, event.id) > (current.created_at, current.id)
        });
        if replace {
            heads.insert(key, event);
        }
    }
    heads
}

fn newest_head<'a>(events: &'a [Event], coordinate: &str) -> Option<&'a Event> {
    events
        .iter()
        .filter(|event| first_tag(event, "d").as_deref() == Some(coordinate))
        .max_by_key(|event| (event.created_at, event.id))
}

fn governed_nodes(events: &[&Event], only_node: Option<&str>) -> BTreeSet<String> {
    if let Some(node) = only_node {
        return BTreeSet::from([node.to_string()]);
    }
    events
        .iter()
        .filter_map(|event| first_tag(event, "n"))
        .collect()
}

fn status(event: &Event) -> String {
    serde_json::from_str::<Value>(&event.content)
        .ok()
        .and_then(|content| {
            content
                .get("status")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .unwrap_or_else(|| "active".into())
}

fn first_tag(event: &Event, name: &str) -> Option<String> {
    tag_values(event, name).into_iter().next()
}

fn tag_values(event: &Event, name: &str) -> Vec<String> {
    event
        .tags
        .iter()
        .filter_map(|tag| {
            let values = tag.as_slice();
            (values.first().map(String::as_str) == Some(name))
                .then(|| values.get(1).cloned())
                .flatten()
        })
        .collect()
}

fn validate_stream(stream: &str) -> Result<(), CliError> {
    if stream.is_empty()
        || stream.split('/').any(|segment| {
            segment.is_empty()
                || matches!(segment, "." | "..")
                || !segment.bytes().all(|byte| {
                    byte.is_ascii_lowercase()
                        || byte.is_ascii_digit()
                        || matches!(byte, b'-' | b'_' | b'.')
                })
        })
    {
        return Err(CliError::Usage(
            "stream must use lowercase slug segments separated by '/'".into(),
        ));
    }
    Ok(())
}

fn validate_principal(principal: &str) -> Result<(), CliError> {
    if principal.is_empty()
        || principal
            .chars()
            .any(|character| character.is_control() || character.is_whitespace())
    {
        return Err(CliError::Usage(
            "principal must be a nonempty, whitespace-free stable label".into(),
        ));
    }
    Ok(())
}

fn validate_pubkeys(values: &[String], label: &str) -> Result<(), CliError> {
    if values.is_empty() {
        return Err(CliError::Usage(format!(
            "at least one {label} public key is required"
        )));
    }
    for value in values {
        if value.len() != 64 || value.to_ascii_lowercase() != *value {
            return Err(CliError::Usage(format!(
                "{label} public key must be 64 lowercase hexadecimal characters"
            )));
        }
        PublicKey::from_hex(value)
            .map_err(|error| CliError::Usage(format!("invalid {label} public key: {error}")))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn streams_are_path_safe_but_keep_shared_namespace() {
        assert!(validate_stream("shared/tooling").is_ok());
        assert!(validate_stream("../tooling").is_err());
        assert!(validate_stream("shared//tooling").is_err());
    }
}
