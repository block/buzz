use buzz_core::kind::KIND_IA_ARCHIVED_LIST;
use buzz_sdk::builders::{build_archive_identity_request, build_unarchive_identity_request};
use nostr::PublicKey;
use serde_json::json;

use crate::agent_management::{build_create, build_update, CreateAgentDraft, UpdateAgentDraft};
use crate::client::BuzzClient;
use crate::error::CliError;
use crate::validate::{read_or_stdin, validate_hex64};
use crate::{AgentsCmd, RespondToArg};

pub async fn dispatch(command: AgentsCmd, client: &BuzzClient) -> Result<(), CliError> {
    match command {
        AgentsCmd::DraftCreate {
            channel,
            display_name,
            system_prompt,
        } => {
            let owner = require_owner(client)?;
            let built = build_create(
                client.keys(),
                &owner,
                CreateAgentDraft {
                    channel_id: channel,
                    display_name,
                    system_prompt: read_or_stdin(&system_prompt)?,
                },
            )?;
            let response = client.publish_ephemeral_event(built.event).await?;
            let mut output: serde_json::Value = serde_json::from_str(&response)
                .map_err(|e| CliError::Other(format!("invalid relay response: {e}")))?;
            if let Some(obj) = output.as_object_mut() {
                obj.insert("request_id".into(), built.request_id.into());
                obj.insert("action".into(), built.action.into());
                obj.insert("saved".into(), false.into());
                obj.insert(
                    "message".into(),
                    "Draft sent to Buzz Desktop for owner review. Nothing changes until the owner saves it."
                        .into(),
                );
            }
            println!("{output}");
            Ok(())
        }

        AgentsCmd::DraftUpdate {
            channel,
            agent_name,
            display_name,
            system_prompt,
            runtime,
            provider,
            model,
            respond_to,
        } => {
            let owner = require_owner(client)?;
            let built = build_update(
                client.keys(),
                &owner,
                UpdateAgentDraft {
                    channel_id: channel,
                    agent_name,
                    display_name,
                    system_prompt: system_prompt.map(|v| read_or_stdin(&v)).transpose()?,
                    runtime,
                    provider,
                    model,
                    respond_to: respond_to.map(RespondToArg::to_wire),
                },
            )?;
            let response = client.publish_ephemeral_event(built.event).await?;
            let mut output: serde_json::Value = serde_json::from_str(&response)
                .map_err(|e| CliError::Other(format!("invalid relay response: {e}")))?;
            if let Some(obj) = output.as_object_mut() {
                obj.insert("request_id".into(), built.request_id.into());
                obj.insert("action".into(), built.action.into());
                obj.insert("saved".into(), false.into());
                obj.insert(
                    "message".into(),
                    "Draft sent to Buzz Desktop for owner review. Nothing changes until the owner saves it."
                        .into(),
                );
            }
            println!("{output}");
            Ok(())
        }

        AgentsCmd::Archive {
            target_pubkey,
            reason,
            replaced_by,
            content,
        } => {
            validate_hex64(&target_pubkey)?;
            let signer_hex = client.keys().public_key().to_hex();
            let auth = resolve_auth(client, &target_pubkey, &signer_hex).await?;
            let builder = build_archive_identity_request(
                &target_pubkey,
                &content,
                reason.as_deref(),
                replaced_by.as_deref(),
                auth.as_ref(),
            )
            .map_err(|e| CliError::Usage(format!("invalid archive request: {e}")))?;
            let event = client.sign_event_unchecked(builder)?;
            let event_id = event.id.to_hex();
            client.submit_event(event).await?;
            println!(
                "{}",
                json!({
                    "ok": true,
                    "event_id": event_id,
                    "action": "archive",
                    "target": target_pubkey,
                })
            );
            Ok(())
        }

        AgentsCmd::Unarchive {
            target_pubkey,
            reason,
            content,
        } => {
            validate_hex64(&target_pubkey)?;
            let signer_hex = client.keys().public_key().to_hex();
            let auth = resolve_auth(client, &target_pubkey, &signer_hex).await?;
            let builder = build_unarchive_identity_request(
                &target_pubkey,
                &content,
                reason.as_deref(),
                auth.as_ref(),
            )
            .map_err(|e| CliError::Usage(format!("invalid unarchive request: {e}")))?;
            let event = client.sign_event_unchecked(builder)?;
            let event_id = event.id.to_hex();
            client.submit_event(event).await?;
            println!(
                "{}",
                json!({
                    "ok": true,
                    "event_id": event_id,
                    "action": "unarchive",
                    "target": target_pubkey,
                })
            );
            Ok(())
        }

        AgentsCmd::Archived => cmd_archived(client).await,
    }
}

/// Require `BUZZ_AUTH_TAG` and parse the owner pubkey from it. Used only by
/// the `draft-create` and `draft-update` paths.
fn require_owner(client: &BuzzClient) -> Result<PublicKey, CliError> {
    let hex = client
        .auth_tag_owner_hex()
        .ok_or_else(|| CliError::Auth("agent draft requests require BUZZ_AUTH_TAG".into()))?;
    PublicKey::parse(&hex).map_err(|e| CliError::Auth(format!("invalid owner attestation: {e}")))
}

/// Resolve the optional NIP-OA `auth` tag for archive/unarchive requests.
///
/// Mirrors the desktop's `maybe_owner_auth_tag`:
/// - `target == signer`: self path — no auth needed → `Ok(None)`.
/// - Otherwise: fetch target's kind:0, look for an `auth` tag whose owner
///   (index 1) matches the signer. Return it when present; `Ok(None)` when
///   absent or structurally malformed. Query/network failures surface as
///   `Err` — silent degradation to bare would make the relay reject the
///   request with a misleading error.
async fn resolve_auth(
    client: &BuzzClient,
    target_hex: &str,
    signer_hex: &str,
) -> Result<Option<[String; 4]>, CliError> {
    if target_hex.eq_ignore_ascii_case(signer_hex) {
        return Ok(None);
    }
    let filter = json!({"kinds": [0], "authors": [target_hex], "limit": 1});
    let raw = client
        .query(&filter)
        .await
        .map_err(|e| CliError::Other(format!("failed to fetch target kind:0: {e}")))?;
    let events: Vec<serde_json::Value> = serde_json::from_str(&raw)
        .map_err(|e| CliError::Other(format!("invalid kind:0 query response: {e}")))?;
    let event = match events.into_iter().next() {
        Some(e) => e,
        None => return Ok(None),
    };
    let tags = match event.get("tags").and_then(|v| v.as_array()) {
        Some(t) => t,
        None => return Ok(None),
    };
    Ok(extract_owner_auth_tag(tags, signer_hex))
}

/// Pure extraction helper: require exactly one kind:0 tag whose first
/// element is `"auth"` (a set-level rule — a valid tag alongside a second
/// malformed or duplicate `auth`-labeled tag is bare, not the valid one),
/// then structurally validate that sole tag as
/// `["auth", owner, conditions, sig]` matching `signer_hex`.
///
/// Malformed tags (wrong arity, non-string elements, non-hex fields) are
/// silently skipped — the contract is "bare" (None), not error.
fn extract_owner_auth_tag(tags: &[serde_json::Value], signer_hex: &str) -> Option<[String; 4]> {
    let auth_tags: Vec<&serde_json::Value> = tags
        .iter()
        .filter(|tag| {
            tag.as_array()
                .and_then(|elems| elems.first())
                .and_then(|v| v.as_str())
                == Some("auth")
        })
        .collect();
    if auth_tags.len() != 1 {
        return None;
    }

    let elems = auth_tags[0].as_array()?;
    if elems.len() != 4 {
        return None;
    }
    let label = elems[0].as_str()?;
    let owner = elems[1].as_str()?;
    if !owner.eq_ignore_ascii_case(signer_hex) {
        return None;
    }
    let conditions = elems[2].as_str()?;
    let sig = elems[3].as_str()?;
    if owner.len() != 64
        || !owner.chars().all(|c| c.is_ascii_hexdigit())
        || sig.len() != 128
        || !sig.chars().all(|c| c.is_ascii_hexdigit())
    {
        return None;
    }
    Some([
        label.to_owned(),
        owner.to_owned(),
        conditions.to_owned(),
        sig.to_owned(),
    ])
}

/// Validate the NIP-11 relay-info `self` field is a 64-hex pubkey and
/// normalize it to lowercase, so the archived-identities query filter and
/// the author comparison in [`verify_archived_event`] agree regardless of
/// the case the relay published `self` in.
fn normalize_relay_self_hex(self_hex: &str) -> Result<String, CliError> {
    if self_hex.len() != 64 || !self_hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(CliError::Other(format!(
            "relay 'self' field is not a valid 64-hex pubkey: {self_hex}"
        )));
    }
    Ok(self_hex.to_ascii_lowercase())
}

/// Fetch and verify the relay's NIP-IA archived-identities snapshot (kind
/// 13535). Shared by `cmd_archived` (trust failures are fatal — verifying
/// repair state is the command's whole purpose) and the `--template`
/// resolver's archive filter, which fails open on a trust failure instead
/// (see `channels::resolve_roster_with_archive_filter`'s doc comment for
/// why).
///
/// Three trust states:
/// - State 1: no events — `Ok(vec![])`
/// - State 2: event passes all checks — `Ok(<pubkeys>)`
/// - State 3: trust failure — `Err`, naming the specific failure
pub(crate) async fn fetch_archived_snapshot(client: &BuzzClient) -> Result<Vec<String>, CliError> {
    // Fetch NIP-11 info to get the relay's self pubkey.
    let nip11_raw = client
        .get_public("/")
        .await
        .map_err(|e| CliError::Other(format!("failed to fetch relay info document: {e}")))?;
    let nip11: serde_json::Value = serde_json::from_str(&nip11_raw)
        .map_err(|e| CliError::Other(format!("relay info document is not valid JSON: {e}")))?;
    let self_hex = nip11
        .get("self")
        .and_then(|v| v.as_str())
        .ok_or_else(|| CliError::Other("relay info document missing 'self' field".into()))?;
    let self_hex = normalize_relay_self_hex(self_hex)?;

    // Query for the archived-identities list.
    let filter = json!({"kinds": [KIND_IA_ARCHIVED_LIST], "authors": [self_hex], "limit": 1});
    let raw = client
        .query(&filter)
        .await
        .map_err(|e| CliError::Other(format!("failed to query archived-identities list: {e}")))?;
    let events: Vec<serde_json::Value> = serde_json::from_str(&raw)
        .map_err(|e| CliError::Other(format!("invalid query response: {e}")))?;

    // State 1: no events.
    if events.is_empty() {
        return Ok(Vec::new());
    }

    // State 2 or 3: verify then collect.
    let raw_event = events.into_iter().next().unwrap();
    let event: nostr::Event = serde_json::from_value(raw_event)
        .map_err(|e| CliError::Other(format!("archived-identities event is malformed: {e}")))?;
    let archived = verify_archived_event(&event, &self_hex)?;

    Ok(archived.into_iter().map(str::to_string).collect())
}

/// `buzz agents archived`: read path over [`fetch_archived_snapshot`] for
/// direct invocation — a trust failure (state 3) is fatal here so a
/// verification command can never look like success.
async fn cmd_archived(client: &BuzzClient) -> Result<(), CliError> {
    let archived = fetch_archived_snapshot(client).await?;
    println!("{}", json!({"archived": archived}));
    Ok(())
}

/// Pure verification of a kind:13535 archived-identities event.
///
/// Returns the list of valid hex64 pubkeys from `p` tags on success, or a
/// named trust-failure error (State 3).
fn verify_archived_event<'a>(
    event: &'a nostr::Event,
    relay_self_hex: &str,
) -> Result<Vec<&'a str>, CliError> {
    if event.kind != nostr::Kind::Custom(KIND_IA_ARCHIVED_LIST as u16) {
        return Err(CliError::Other(format!(
            "archived-identities event has wrong kind: {}",
            event.kind.as_u16()
        )));
    }

    if event.pubkey.to_hex() != relay_self_hex {
        return Err(CliError::Other(format!(
            "archived-identities event author {} does not match relay self {}",
            event.pubkey.to_hex(),
            relay_self_hex
        )));
    }

    let mut nip70_count = 0usize;
    for t in event.tags.iter() {
        let s = t.as_slice();
        if s.first().map(String::as_str) != Some("-") {
            continue;
        }
        if s.len() != 1 {
            return Err(CliError::Other(
                "archived-identities event has a malformed NIP-70 '-' tag (expected arity 1)"
                    .into(),
            ));
        }
        nip70_count += 1;
    }
    if nip70_count != 1 {
        return Err(CliError::Other(format!(
            "archived-identities event must have exactly one NIP-70 '-' tag, found {nip70_count}"
        )));
    }

    event.verify().map_err(|e| {
        CliError::Other(format!(
            "archived-identities event failed cryptographic verification: {e}"
        ))
    })?;

    let archived: Vec<&str> = event
        .tags
        .iter()
        .filter_map(|t| {
            let s = t.as_slice();
            if s.first().map(String::as_str) == Some("p") {
                let pk = s.get(1).map(String::as_str)?;
                if pk.len() == 64 && pk.chars().all(|c| c.is_ascii_hexdigit()) {
                    return Some(pk);
                }
            }
            None
        })
        .collect();

    Ok(archived)
}

#[cfg(test)]
mod tests {
    use super::*;
    use buzz_core::kind::KIND_IA_ARCHIVED_LIST;
    use nostr::{EventBuilder, Keys, Kind, Tag};
    use serde_json::json;

    fn hex64(c: char) -> String {
        std::iter::repeat_n(c, 64).collect()
    }

    fn hex128(c: char) -> String {
        std::iter::repeat_n(c, 128).collect()
    }

    // --- (b) auth-selection matrix: extract_owner_auth_tag ---

    #[test]
    fn auth_selection_owner_match_returns_tag() {
        let signer = hex64('a');
        let sig = hex128('b');
        let tags = vec![json!(["auth", signer, "conditions", sig])];
        let result = extract_owner_auth_tag(&tags, &signer);
        assert!(result.is_some());
        let tag = result.unwrap();
        assert_eq!(tag[0], "auth");
        assert_eq!(tag[1], signer);
        assert_eq!(tag[2], "conditions");
        assert_eq!(tag[3], sig);
    }

    #[test]
    fn auth_selection_non_owner_returns_none() {
        let signer = hex64('a');
        let other_owner = hex64('b');
        let tags = vec![json!(["auth", other_owner, "", hex128('c')])];
        assert!(extract_owner_auth_tag(&tags, &signer).is_none());
    }

    #[test]
    fn auth_selection_malformed_three_elements_returns_none() {
        let signer = hex64('a');
        let tags = vec![json!(["auth", signer, "conditions"])];
        assert!(extract_owner_auth_tag(&tags, &signer).is_none());
    }

    #[test]
    fn auth_selection_malformed_five_elements_returns_none() {
        let signer = hex64('a');
        let tags = vec![json!(["auth", signer, "conditions", hex128('b'), "extra"])];
        assert!(extract_owner_auth_tag(&tags, &signer).is_none());
    }

    #[test]
    fn auth_selection_malformed_non_hex_owner_returns_none() {
        let signer = "z".repeat(64);
        let tags = vec![json!(["auth", signer, "", hex128('a')])];
        assert!(extract_owner_auth_tag(&tags, &signer).is_none());
    }

    #[test]
    fn auth_selection_malformed_non_hex_sig_returns_none() {
        let signer = hex64('a');
        let bad_sig = "z".repeat(128);
        let tags = vec![json!(["auth", signer, "", bad_sig])];
        assert!(extract_owner_auth_tag(&tags, &signer).is_none());
    }

    #[test]
    fn auth_selection_malformed_short_sig_returns_none() {
        let signer = hex64('a');
        let short_sig = hex128('a')[..64].to_string();
        let tags = vec![json!(["auth", signer, "", short_sig])];
        assert!(extract_owner_auth_tag(&tags, &signer).is_none());
    }

    #[test]
    fn auth_selection_case_insensitive_owner_match() {
        let signer_lower = hex64('a');
        let signer_upper = signer_lower.to_uppercase();
        let sig = hex128('b');
        let tags = vec![json!(["auth", signer_upper, "cond", sig])];
        let result = extract_owner_auth_tag(&tags, &signer_lower);
        assert!(result.is_some());
    }

    #[test]
    fn auth_selection_non_string_elements_returns_none() {
        let signer = hex64('a');
        let tags = vec![json!(["auth", signer, 42, hex128('b')])];
        assert!(extract_owner_auth_tag(&tags, &signer).is_none());
    }

    #[test]
    fn auth_selection_non_array_tag_skipped() {
        let signer = hex64('a');
        let tags = vec![
            json!("not an array"),
            json!(["auth", signer, "", hex128('b')]),
        ];
        let result = extract_owner_auth_tag(&tags, &signer);
        assert!(result.is_some());
    }

    #[test]
    fn auth_selection_no_tags_returns_none() {
        assert!(extract_owner_auth_tag(&[], &hex64('a')).is_none());
    }

    #[test]
    fn auth_selection_wrong_label_returns_none() {
        let signer = hex64('a');
        let tags = vec![json!(["delegation", signer, "", hex128('b')])];
        assert!(extract_owner_auth_tag(&tags, &signer).is_none());
    }

    #[test]
    fn auth_selection_valid_plus_duplicate_auth_tag_returns_none() {
        // Set-level rule (F6): a structurally valid, owner-matching `auth`
        // tag alongside a second `auth`-labeled tag (malformed or a
        // duplicate) must not be selected — the whole kind:0 is bare.
        let signer = hex64('a');
        let sig = hex128('b');
        let tags = vec![
            json!(["auth", signer, "conditions", sig]),
            json!(["auth", signer, "conditions", sig]),
        ];
        assert!(extract_owner_auth_tag(&tags, &signer).is_none());
    }

    #[test]
    fn auth_selection_valid_plus_malformed_second_auth_tag_returns_none() {
        let signer = hex64('a');
        let sig = hex128('b');
        let tags = vec![
            json!(["auth", signer, "conditions", sig]),
            json!(["auth", "not-hex", "conditions"]),
        ];
        assert!(extract_owner_auth_tag(&tags, &signer).is_none());
    }

    // --- (d) NIP-11 self normalization: normalize_relay_self_hex ---

    #[test]
    fn normalize_self_lowercases_uppercase_hex() {
        let upper = hex64('A');
        let result = normalize_relay_self_hex(&upper).expect("should pass");
        assert_eq!(result, hex64('a'));
    }

    #[test]
    fn normalize_self_rejects_wrong_length() {
        assert!(normalize_relay_self_hex(&hex64('a')[..63]).is_err());
    }

    #[test]
    fn normalize_self_rejects_non_hex() {
        assert!(normalize_relay_self_hex(&"z".repeat(64)).is_err());
    }

    #[test]
    fn archived_uppercase_self_matches_lowercase_event_author() {
        // F7: an uppercase NIP-11 `self` must still resolve to the same
        // relay identity as the event's (always-lowercase) author hex once
        // normalized — before the fix this was a case-sensitive mismatch.
        let keys = Keys::generate();
        let self_hex_lower = keys.public_key().to_hex();
        let self_hex_upper = self_hex_lower.to_uppercase();
        let normalized = normalize_relay_self_hex(&self_hex_upper).expect("valid hex");
        let event = build_archived_event(&keys, KIND_IA_ARCHIVED_LIST as u16, &[], true);
        let result = verify_archived_event(&event, &normalized).expect("should pass");
        assert!(result.is_empty());
    }

    // --- (c) snapshot tri-state: verify_archived_event ---

    fn build_archived_event(
        keys: &Keys,
        kind: u16,
        p_tags: &[&str],
        include_nip70: bool,
    ) -> nostr::Event {
        let mut tags: Vec<Tag> = Vec::new();
        if include_nip70 {
            tags.push(Tag::parse(["-"]).unwrap());
        }
        for pk in p_tags {
            tags.push(Tag::parse(["p", pk]).unwrap());
        }
        EventBuilder::new(Kind::Custom(kind), "")
            .tags(tags)
            .sign_with_keys(keys)
            .expect("sign")
    }

    #[test]
    fn archived_state2_valid_event_returns_pubkeys() {
        let keys = Keys::generate();
        let self_hex = keys.public_key().to_hex();
        let pk1 = hex64('a');
        let pk2 = hex64('b');
        let event = build_archived_event(&keys, KIND_IA_ARCHIVED_LIST as u16, &[&pk1, &pk2], true);
        let result = verify_archived_event(&event, &self_hex).expect("should pass");
        assert_eq!(result, vec![pk1.as_str(), pk2.as_str()]);
    }

    #[test]
    fn archived_state2_empty_p_tags_returns_empty() {
        let keys = Keys::generate();
        let self_hex = keys.public_key().to_hex();
        let event = build_archived_event(&keys, KIND_IA_ARCHIVED_LIST as u16, &[], true);
        let result = verify_archived_event(&event, &self_hex).expect("should pass");
        assert!(result.is_empty());
    }

    #[test]
    fn archived_state3_wrong_kind_errors() {
        let keys = Keys::generate();
        let self_hex = keys.public_key().to_hex();
        let event = build_archived_event(&keys, 9999, &[], true);
        let err = verify_archived_event(&event, &self_hex).unwrap_err();
        assert!(
            err.to_string().contains("wrong kind"),
            "error should name wrong kind: {err}"
        );
    }

    #[test]
    fn archived_state3_wrong_author_errors() {
        let event_keys = Keys::generate();
        let other_self = hex64('f');
        let event = build_archived_event(&event_keys, KIND_IA_ARCHIVED_LIST as u16, &[], true);
        let err = verify_archived_event(&event, &other_self).unwrap_err();
        assert!(
            err.to_string().contains("does not match relay self"),
            "error should name author mismatch: {err}"
        );
    }

    #[test]
    fn archived_state3_no_nip70_tag_errors() {
        let keys = Keys::generate();
        let self_hex = keys.public_key().to_hex();
        let event = build_archived_event(&keys, KIND_IA_ARCHIVED_LIST as u16, &[], false);
        let err = verify_archived_event(&event, &self_hex).unwrap_err();
        assert!(
            err.to_string().contains("NIP-70"),
            "error should name missing NIP-70 tag: {err}"
        );
    }

    #[test]
    fn archived_state3_duplicate_nip70_tags_errors() {
        let keys = Keys::generate();
        let self_hex = keys.public_key().to_hex();
        let event = EventBuilder::new(Kind::Custom(KIND_IA_ARCHIVED_LIST as u16), "")
            .tags([Tag::parse(["-"]).unwrap(), Tag::parse(["-"]).unwrap()])
            .sign_with_keys(&keys)
            .expect("sign");
        let err = verify_archived_event(&event, &self_hex).unwrap_err();
        assert!(
            err.to_string().contains("found 2"),
            "error should report 2 NIP-70 tags: {err}"
        );
    }

    #[test]
    fn archived_state3_lone_malformed_nip70_tag_errors() {
        let keys = Keys::generate();
        let self_hex = keys.public_key().to_hex();
        let event = EventBuilder::new(Kind::Custom(KIND_IA_ARCHIVED_LIST as u16), "")
            .tags([Tag::parse(["-", "extra"]).unwrap()])
            .sign_with_keys(&keys)
            .expect("sign");
        let err = verify_archived_event(&event, &self_hex).unwrap_err();
        assert!(
            err.to_string().contains("malformed NIP-70"),
            "error should name the malformed NIP-70 tag: {err}"
        );
    }

    #[test]
    fn archived_state3_exact_marker_plus_malformed_marker_errors() {
        // F5 (IMPORTANT, discriminating): a valid `["-"]` alongside a
        // malformed `["-", "extra"]` must still poison the snapshot — the
        // old count-of-exact-shape-only check let this bypass through with
        // nip70_count == 1.
        let keys = Keys::generate();
        let self_hex = keys.public_key().to_hex();
        let event = EventBuilder::new(Kind::Custom(KIND_IA_ARCHIVED_LIST as u16), "")
            .tags([
                Tag::parse(["-"]).unwrap(),
                Tag::parse(["-", "extra"]).unwrap(),
            ])
            .sign_with_keys(&keys)
            .expect("sign");
        let err = verify_archived_event(&event, &self_hex).unwrap_err();
        assert!(
            err.to_string().contains("malformed NIP-70"),
            "error should name the malformed NIP-70 tag: {err}"
        );
    }

    #[test]
    fn archived_non_hex_p_tag_dropped() {
        let keys = Keys::generate();
        let self_hex = keys.public_key().to_hex();
        let valid_pk = hex64('a');
        let event = EventBuilder::new(Kind::Custom(KIND_IA_ARCHIVED_LIST as u16), "")
            .tags([
                Tag::parse(["-"]).unwrap(),
                Tag::parse(["p", &valid_pk]).unwrap(),
                Tag::parse(["p", "not-hex-at-all"]).unwrap(),
                Tag::parse(["p", &"z".repeat(64)]).unwrap(),
            ])
            .sign_with_keys(&keys)
            .expect("sign");
        let result = verify_archived_event(&event, &self_hex).expect("should pass");
        assert_eq!(result, vec![valid_pk.as_str()]);
    }

    #[test]
    fn archived_short_p_tag_dropped() {
        let keys = Keys::generate();
        let self_hex = keys.public_key().to_hex();
        let event = EventBuilder::new(Kind::Custom(KIND_IA_ARCHIVED_LIST as u16), "")
            .tags([
                Tag::parse(["-"]).unwrap(),
                Tag::parse(["p", &hex64('a')[..32]]).unwrap(),
            ])
            .sign_with_keys(&keys)
            .expect("sign");
        let result = verify_archived_event(&event, &self_hex).expect("should pass");
        assert!(result.is_empty());
    }
}
