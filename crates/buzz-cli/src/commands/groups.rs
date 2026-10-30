//! `buzz groups` — community user groups (NIP-UG, kind:30181), the Buzz
//! equivalent of a Slack user group such as `@ios-team`.
//!
//! A group is one addressable event per handle. Every write is one strongly
//! consistent read of the current head followed by one publish, so a change
//! never builds on a lagging replica. The relay decides who may edit.

use std::time::SystemTime;

use buzz_core::kind::KIND_USER_GROUP;
use buzz_core::user_group::{
    normalize_handle, parse_user_group, resolve_head, EditPolicy, UserGroup,
    MAX_GROUP_MENTIONS_PER_EVENT,
};
use nostr::{Event, PublicKey, Timestamp};
use serde_json::{json, Value};

use super::parse_write_response;
use crate::client::BuzzClient;
use crate::error::CliError;
use crate::{GroupPolicy, GroupsCmd};

const LIST_LIMIT: u64 = 500;

struct Head {
    event: Event,
    group: UserGroup,
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn handle_arg(raw: &str) -> Result<String, CliError> {
    normalize_handle(raw).map_err(|e| CliError::Usage(e.to_string()))
}

fn pubkey_arg(client: &BuzzClient, raw: &str) -> Result<String, CliError> {
    let raw = raw.trim();
    if raw.eq_ignore_ascii_case("me") {
        return Ok(client.keys().public_key().to_hex());
    }
    PublicKey::parse(raw)
        .map(|pk| pk.to_hex())
        .map_err(|_| CliError::Usage(format!("invalid pubkey (use hex, npub, or `me`): {raw}")))
}

fn pubkeys_arg(client: &BuzzClient, raw: &[String]) -> Result<Vec<String>, CliError> {
    let mut out: Vec<String> = Vec::new();
    for value in raw {
        let pubkey = pubkey_arg(client, value)?;
        if !out.contains(&pubkey) {
            out.push(pubkey);
        }
    }
    Ok(out)
}

/// The live heads that match `filter`, one per handle. Events that fail
/// signature or NIP-UG validation are skipped.
async fn fetch_heads(client: &BuzzClient, mut filter: Value) -> Result<Vec<Head>, CliError> {
    filter["kinds"] = json!([KIND_USER_GROUP]);
    filter["consistency"] = json!("strong");
    let raw = client.query(&filter).await?;
    let values: Vec<Value> = serde_json::from_str(&raw)
        .map_err(|e| CliError::Other(format!("failed to parse user group query: {e}")))?;
    let events: Vec<Event> = values
        .into_iter()
        .filter_map(|v| serde_json::from_value::<Event>(v).ok())
        .filter(|e| e.verify().is_ok())
        .collect();
    let mut handles: Vec<String> = events
        .iter()
        .filter_map(|e| parse_user_group(e).ok().map(|g| g.handle))
        .collect();
    handles.sort();
    handles.dedup();
    Ok(handles
        .into_iter()
        .filter_map(|handle| {
            resolve_head(&handle, &events).map(|(event, group)| Head {
                event: event.clone(),
                group,
            })
        })
        .collect())
}

async fn fetch_head(client: &BuzzClient, handle: &str) -> Result<Option<Head>, CliError> {
    let heads = fetch_heads(client, json!({"#d": [handle], "limit": 50})).await?;
    Ok(heads.into_iter().find(|h| h.group.handle == handle))
}

async fn require_head(client: &BuzzClient, handle: &str) -> Result<Head, CliError> {
    fetch_head(client, handle)
        .await?
        .ok_or_else(|| CliError::NotFound(format!("no user group @{handle}")))
}

fn describe(head: &Head) -> Value {
    let g = &head.group;
    json!({
        "handle": g.handle,
        "name": g.name,
        "description": g.description,
        "creator": g.creator,
        "admins": g.admins,
        "members": g.members,
        "member_count": g.members.len(),
        "policy": g.policy.as_str(),
        "status": if g.disabled { "disabled" } else { "active" },
        "event_id": head.event.id.to_hex(),
        "author": head.event.pubkey.to_hex(),
        "updated_at": head.event.created_at.as_secs(),
    })
}

fn summarize(head: &Head) -> Value {
    let mut value = describe(head);
    if let Some(object) = value.as_object_mut() {
        object.remove("members");
        object.remove("admins");
    }
    value
}

fn policy(policy: GroupPolicy) -> EditPolicy {
    match policy {
        GroupPolicy::Admins => EditPolicy::Admins,
        GroupPolicy::Open => EditPolicy::Open,
    }
}

/// Signs and submits `group` as the new head. A write over an existing head is
/// stamped one second after it, so another author's edit is always newer.
async fn publish(
    client: &BuzzClient,
    group: &UserGroup,
    after: Option<&Head>,
) -> Result<Value, CliError> {
    let created_at = after.map_or(0, |h| h.event.created_at.as_secs() + 1);
    let builder = buzz_sdk::build_user_group(group)
        .map_err(|e| CliError::Usage(e.to_string()))?
        .custom_created_at(Timestamp::from(now_secs().max(created_at)));
    let raw = client.submit_event(client.sign_event(builder)?).await?;
    let normalized = parse_write_response(
        &raw,
        "a newer change to this group won; run the command again",
    )?;
    let mut response: Value = serde_json::from_str(&normalized)
        .map_err(|e| CliError::Other(format!("relay response is not JSON: {e}")))?;
    response["handle"] = json!(group.handle);
    response["member_count"] = json!(group.members.len());
    Ok(response)
}

/// Applies `change` to the current head of `handle` and publishes the result.
/// A change that leaves the group as it was publishes nothing.
async fn edit(
    client: &BuzzClient,
    handle: &str,
    change: impl FnOnce(&mut UserGroup) -> Result<(), CliError>,
) -> Result<Value, CliError> {
    let head = require_head(client, handle).await?;
    let mut next = head.group.clone();
    change(&mut next)?;
    if next == head.group {
        return Ok(json!({
            "event_id": null,
            "accepted": true,
            "message": "unchanged",
            "handle": handle,
            "member_count": next.members.len(),
        }));
    }
    publish(client, &next, Some(&head)).await
}

fn require_members(members: &[String]) -> Result<(), CliError> {
    if members.is_empty() {
        return Err(CliError::Usage("pass at least one --member".into()));
    }
    Ok(())
}

async fn run(cmd: GroupsCmd, client: &BuzzClient) -> Result<Value, CliError> {
    match cmd {
        GroupsCmd::List {
            member,
            include_disabled,
        } => {
            let mut filter = json!({"limit": LIST_LIMIT});
            if let Some(member) = member {
                filter["#p"] = json!([pubkey_arg(client, &member)?]);
            }
            let mut heads = fetch_heads(client, filter).await?;
            heads.retain(|h| include_disabled || !h.group.disabled);
            heads.sort_by(|a, b| a.group.handle.cmp(&b.group.handle));
            Ok(Value::Array(heads.iter().map(summarize).collect()))
        }
        GroupsCmd::Get { handle } => {
            let handle = handle_arg(&handle)?;
            Ok(describe(&require_head(client, &handle).await?))
        }
        GroupsCmd::Members { handle } => {
            let handle = handle_arg(&handle)?;
            let head = require_head(client, &handle).await?;
            Ok(json!({
                "handle": handle,
                "members": head.group.members,
                "member_count": head.group.members.len(),
            }))
        }
        GroupsCmd::Create {
            handle,
            name,
            description,
            members,
            admins,
            policy: edit_policy,
        } => {
            let handle = handle_arg(&handle)?;
            if fetch_head(client, &handle).await?.is_some() {
                return Err(CliError::Conflict(format!(
                    "user group @{handle} already exists; use `buzz groups update`"
                )));
            }
            let group = UserGroup {
                handle,
                name: name.trim().to_string(),
                description: description.unwrap_or_default(),
                creator: client.keys().public_key().to_hex(),
                admins: pubkeys_arg(client, &admins)?,
                members: pubkeys_arg(client, &members)?,
                policy: policy(edit_policy),
                disabled: false,
            };
            publish(client, &group, None).await
        }
        GroupsCmd::Update {
            handle,
            name,
            description,
            policy: new_policy,
            add_admins,
            remove_admins,
            set_members,
            clear_members,
            disable,
            enable,
        } => {
            let handle = handle_arg(&handle)?;
            let add_admins = pubkeys_arg(client, &add_admins)?;
            let remove_admins = pubkeys_arg(client, &remove_admins)?;
            let set_members = pubkeys_arg(client, &set_members)?;
            if name.is_none()
                && description.is_none()
                && new_policy.is_none()
                && add_admins.is_empty()
                && remove_admins.is_empty()
                && set_members.is_empty()
                && !clear_members
                && !disable
                && !enable
            {
                return Err(CliError::Usage(
                    "pass at least one of --name, --description, --policy, --add-admin, \
                     --remove-admin, --set-member, --clear-members, --disable, --enable"
                        .into(),
                ));
            }
            edit(client, &handle, |group| {
                if let Some(name) = name {
                    group.name = name.trim().to_string();
                }
                if let Some(description) = description {
                    group.description = description;
                }
                if let Some(new_policy) = new_policy {
                    group.policy = policy(new_policy);
                }
                for admin in add_admins {
                    if !group.admins.contains(&admin) {
                        group.admins.push(admin);
                    }
                }
                group.admins.retain(|a| !remove_admins.contains(a));
                if clear_members {
                    group.members.clear();
                }
                if !set_members.is_empty() {
                    group.members = set_members;
                }
                if disable {
                    group.disabled = true;
                }
                if enable {
                    group.disabled = false;
                }
                Ok(())
            })
            .await
        }
        GroupsCmd::AddMember { handle, members } => {
            require_members(&members)?;
            let handle = handle_arg(&handle)?;
            let members = pubkeys_arg(client, &members)?;
            edit(client, &handle, |group| {
                for member in members {
                    if !group.members.contains(&member) {
                        group.members.push(member);
                    }
                }
                Ok(())
            })
            .await
        }
        GroupsCmd::RemoveMember { handle, members } => {
            require_members(&members)?;
            let handle = handle_arg(&handle)?;
            let members = pubkeys_arg(client, &members)?;
            edit(client, &handle, |group| {
                group.members.retain(|m| !members.contains(m));
                Ok(())
            })
            .await
        }
        GroupsCmd::Delete { handle } => {
            let handle = handle_arg(&handle)?;
            let head = require_head(client, &handle).await?;
            let me = client.keys().public_key();
            if head.event.pubkey != me {
                return Err(CliError::Usage(format!(
                    "only the author of the current @{handle} head can delete it; \
                     run `buzz groups update --handle {handle} --disable` to turn it off"
                )));
            }
            let builder = buzz_sdk::build_user_group_deletion(&me.to_hex(), &handle)
                .map_err(|e| CliError::Usage(e.to_string()))?;
            let raw = client.submit_event(client.sign_event(builder)?).await?;
            let normalized = parse_write_response(&raw, "the deletion was already recorded")?;
            let mut response: Value = serde_json::from_str(&normalized)
                .map_err(|e| CliError::Other(format!("relay response is not JSON: {e}")))?;
            response["handle"] = json!(handle);
            Ok(response)
        }
    }
}

/// Fetch the group named by `--mention-group` and build its marked `p` tags,
/// leaving out pubkeys already in `skip`.
pub async fn mention_tags(
    client: &BuzzClient,
    handles: &[String],
    skip: &[String],
) -> Result<Vec<nostr::Tag>, CliError> {
    let mut handles: Vec<String> = handles
        .iter()
        .map(|h| handle_arg(h))
        .collect::<Result<_, _>>()?;
    handles.sort();
    handles.dedup();
    if handles.len() > MAX_GROUP_MENTIONS_PER_EVENT {
        return Err(CliError::Usage(format!(
            "a message may mention at most {MAX_GROUP_MENTIONS_PER_EVENT} groups"
        )));
    }
    let mut tags = Vec::new();
    for handle in handles {
        let head = require_head(client, &handle).await?;
        let mut group = head.group;
        let had_members = !group.members.is_empty();
        group.members.retain(|m| !skip.contains(m));
        if had_members && group.members.is_empty() {
            continue;
        }
        tags.extend(
            buzz_sdk::group_mention_tags(&group).map_err(|e| CliError::Usage(e.to_string()))?,
        );
    }
    Ok(tags)
}

pub async fn dispatch(cmd: GroupsCmd, client: &BuzzClient) -> Result<(), CliError> {
    let output = run(cmd, client).await?;
    println!("{output}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Bytes, extract::State, routing::post, Json, Router};
    use nostr::Keys;
    use std::sync::{Arc, Mutex};
    use tokio::net::TcpListener;

    #[derive(Default)]
    struct Relay {
        events: Vec<Value>,
        submitted: Vec<Value>,
        filters: Vec<Value>,
        reject: Option<String>,
    }
    type Shared = Arc<Mutex<Relay>>;

    /// Fake relay: `/query` records the filter and returns every stored head
    /// whose `d` tag and `p` tags match it; `/events` stores a group head in
    /// place of any head with the same handle, and a deletion removes the head
    /// it names.
    async fn fake_relay(relay: Shared) -> String {
        let app = Router::new()
            .route(
                "/query",
                post(|State(relay): State<Shared>, body: Bytes| async move {
                    let filters: Vec<Value> = serde_json::from_slice(&body).unwrap();
                    let [filter] = filters.as_slice() else {
                        panic!("one filter expected: {filters:?}")
                    };
                    assert_eq!(filter["kinds"], json!([KIND_USER_GROUP]));
                    assert_eq!(filter["consistency"], json!("strong"));
                    let mut relay = relay.lock().unwrap();
                    relay.filters.push(filter.clone());
                    let wants = |tag: &str, value: &Value| -> Option<Vec<Value>> {
                        value[tag].as_array().cloned()
                    };
                    let d = wants("#d", filter);
                    let p = wants("#p", filter);
                    let matching: Vec<Value> = relay
                        .events
                        .iter()
                        .filter(|e| {
                            let tags = e["tags"].as_array().unwrap();
                            let has = |name: &str, wanted: &Option<Vec<Value>>| {
                                wanted.as_ref().is_none_or(|values| {
                                    tags.iter().any(|t| t[0] == name && values.contains(&t[1]))
                                })
                            };
                            has("d", &d) && has("p", &p)
                        })
                        .cloned()
                        .collect();
                    Json(json!(matching))
                }),
            )
            .route(
                "/events",
                post(|State(relay): State<Shared>, body: Bytes| async move {
                    let event: Value = serde_json::from_slice(&body).unwrap();
                    let mut relay = relay.lock().unwrap();
                    relay.submitted.push(event.clone());
                    if let Some(reason) = relay.reject.clone() {
                        return Json(json!({
                            "event_id": event["id"], "accepted": false, "message": reason
                        }));
                    }
                    if event["kind"] == json!(5) {
                        let coordinate = event["tags"][0][1].as_str().unwrap().to_string();
                        relay.events.retain(|e| {
                            let d = e["tags"][0][1].as_str().unwrap();
                            let author = e["pubkey"].as_str().unwrap();
                            coordinate != format!("{KIND_USER_GROUP}:{author}:{d}")
                        });
                    } else {
                        relay.events.retain(|e| e["tags"][0] != event["tags"][0]);
                        relay.events.push(event.clone());
                    }
                    Json(json!({"event_id": event["id"], "accepted": true, "message": ""}))
                }),
            )
            .with_state(relay);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{addr}")
    }

    async fn setup() -> (BuzzClient, Shared) {
        let relay = Shared::default();
        let url = fake_relay(relay.clone()).await;
        let client = BuzzClient::new(url, Keys::generate(), None, None).unwrap();
        (client, relay)
    }

    fn client_for(relay_url: &str) -> BuzzClient {
        BuzzClient::new(relay_url.to_string(), Keys::generate(), None, None).unwrap()
    }

    fn pk(n: u8) -> String {
        format!("{n:02x}").repeat(32)
    }

    fn create(handle: &str, members: &[String]) -> GroupsCmd {
        GroupsCmd::Create {
            handle: handle.into(),
            name: "iOS Team".into(),
            description: Some("Ships the iOS app".into()),
            members: members.to_vec(),
            admins: vec![],
            policy: GroupPolicy::Admins,
        }
    }

    fn update(handle: &str) -> GroupsCmd {
        GroupsCmd::Update {
            handle: handle.into(),
            name: None,
            description: None,
            policy: None,
            add_admins: vec![],
            remove_admins: vec![],
            set_members: vec![],
            clear_members: false,
            disable: false,
            enable: false,
        }
    }

    fn published(relay: &Shared) -> Vec<Value> {
        relay.lock().unwrap().events.clone()
    }

    fn submitted(relay: &Shared) -> Vec<Value> {
        relay.lock().unwrap().submitted.clone()
    }

    #[tokio::test]
    async fn create_publishes_a_valid_head_signed_by_the_caller() {
        let (client, relay) = setup().await;
        let response = run(create("@iOS-Team", &[pk(1), pk(2), pk(1)]), &client)
            .await
            .unwrap();
        assert_eq!(response["handle"], "ios-team");
        assert_eq!(response["member_count"], 2);
        let events = published(&relay);
        assert_eq!(events.len(), 1);
        let event: Event = serde_json::from_value(events[0].clone()).unwrap();
        assert_eq!(event.pubkey, client.keys().public_key());
        let group = parse_user_group(&event).unwrap();
        assert_eq!(group.handle, "ios-team");
        assert_eq!(group.creator, client.keys().public_key().to_hex());
        assert_eq!(group.members, vec![pk(1), pk(2)]);
        assert_eq!(group.description, "Ships the iOS app");
        assert!(!group.disabled);
    }

    #[tokio::test]
    async fn create_refuses_an_existing_handle_and_bad_input() {
        let (client, relay) = setup().await;
        run(create("ios-team", &[]), &client).await.unwrap();
        let err = run(create("ios-team", &[]), &client).await.unwrap_err();
        assert!(matches!(err, CliError::Conflict(_)), "{err}");
        for handle in ["", "here", "bad handle"] {
            let err = run(create(handle, &[]), &client).await.unwrap_err();
            assert!(matches!(err, CliError::Usage(_)), "{handle}: {err}");
        }
        let err = run(create("other", &["nothex".into()]), &client)
            .await
            .unwrap_err();
        assert!(matches!(err, CliError::Usage(_)), "{err}");
        assert_eq!(submitted(&relay).len(), 1);
    }

    #[tokio::test]
    async fn member_commands_read_the_head_and_publish_a_newer_one() {
        let (client, relay) = setup().await;
        run(create("ios-team", &[pk(1)]), &client).await.unwrap();

        let added = run(
            GroupsCmd::AddMember {
                handle: "ios-team".into(),
                members: vec![pk(2), pk(1)],
            },
            &client,
        )
        .await
        .unwrap();
        assert_eq!(added["member_count"], 2);

        let again = run(
            GroupsCmd::AddMember {
                handle: "ios-team".into(),
                members: vec![pk(2)],
            },
            &client,
        )
        .await
        .unwrap();
        assert_eq!(again["message"], "unchanged");
        assert_eq!(submitted(&relay).len(), 2, "no publish when unchanged");

        let removed = run(
            GroupsCmd::RemoveMember {
                handle: "ios-team".into(),
                members: vec![pk(1)],
            },
            &client,
        )
        .await
        .unwrap();
        assert_eq!(removed["member_count"], 1);

        let members = run(
            GroupsCmd::Members {
                handle: "ios-team".into(),
            },
            &client,
        )
        .await
        .unwrap();
        assert_eq!(members["members"], json!([pk(2)]));

        let events: Vec<Event> = submitted(&relay)
            .into_iter()
            .map(|e| serde_json::from_value(e).unwrap())
            .collect();
        assert!(
            events
                .windows(2)
                .all(|w| w[1].created_at.as_secs() > w[0].created_at.as_secs()),
            "each head is strictly newer than the last"
        );
    }

    #[tokio::test]
    async fn an_edit_by_another_author_keeps_the_creator() {
        let (creator, relay) = setup().await;
        run(create("ios-team", &[pk(1)]), &creator).await.unwrap();
        let url = {
            let relay = relay.clone();
            fake_relay(relay).await
        };
        let editor = client_for(&url);
        run(
            GroupsCmd::AddMember {
                handle: "ios-team".into(),
                members: vec![pk(3)],
            },
            &editor,
        )
        .await
        .unwrap();
        let head = fetch_head(&creator, "ios-team").await.unwrap().unwrap();
        assert_eq!(head.group.creator, creator.keys().public_key().to_hex());
        assert_eq!(head.event.pubkey, editor.keys().public_key());
        assert_eq!(head.group.members, vec![pk(1), pk(3)]);
    }

    #[tokio::test]
    async fn update_changes_fields_and_requires_a_change() {
        let (client, _relay) = setup().await;
        run(create("ios-team", &[pk(1)]), &client).await.unwrap();
        let err = run(update("ios-team"), &client).await.unwrap_err();
        assert!(matches!(err, CliError::Usage(_)), "{err}");

        let mut cmd = update("ios-team");
        if let GroupsCmd::Update {
            name,
            policy,
            add_admins,
            set_members,
            disable,
            ..
        } = &mut cmd
        {
            *name = Some("Mobile".into());
            *policy = Some(GroupPolicy::Open);
            *add_admins = vec![pk(9)];
            *set_members = vec![pk(4), pk(5)];
            *disable = true;
        }
        run(cmd, &client).await.unwrap();
        let got = run(
            GroupsCmd::Get {
                handle: "ios-team".into(),
            },
            &client,
        )
        .await
        .unwrap();
        assert_eq!(got["name"], "Mobile");
        assert_eq!(got["policy"], "open");
        assert_eq!(got["status"], "disabled");
        assert_eq!(got["admins"], json!([pk(9)]));
        assert_eq!(got["members"], json!([pk(4), pk(5)]));
        assert_eq!(got["creator"], client.keys().public_key().to_hex());

        let mut cmd = update("ios-team");
        if let GroupsCmd::Update {
            clear_members,
            enable,
            ..
        } = &mut cmd
        {
            *clear_members = true;
            *enable = true;
        }
        run(cmd, &client).await.unwrap();
        let got = run(
            GroupsCmd::Get {
                handle: "ios-team".into(),
            },
            &client,
        )
        .await
        .unwrap();
        assert_eq!(got["status"], "active");
        assert_eq!(got["member_count"], 0);
    }

    #[tokio::test]
    async fn list_filters_by_member_and_hides_disabled_groups() {
        let (client, _relay) = setup().await;
        run(create("ios-team", &[pk(1)]), &client).await.unwrap();
        run(create("android-team", &[pk(2)]), &client)
            .await
            .unwrap();
        run(create("old-team", &[pk(1)]), &client).await.unwrap();
        let mut off = update("old-team");
        if let GroupsCmd::Update { disable, .. } = &mut off {
            *disable = true;
        }
        run(off, &client).await.unwrap();

        let handles = |value: &Value| -> Vec<String> {
            value
                .as_array()
                .unwrap()
                .iter()
                .map(|g| g["handle"].as_str().unwrap().to_string())
                .collect()
        };
        let all = run(
            GroupsCmd::List {
                member: None,
                include_disabled: false,
            },
            &client,
        )
        .await
        .unwrap();
        assert_eq!(handles(&all), vec!["android-team", "ios-team"]);
        assert!(all[0].get("members").is_none(), "list omits member lists");
        assert_eq!(all[0]["member_count"], 1);

        let mine = run(
            GroupsCmd::List {
                member: Some(pk(1)),
                include_disabled: true,
            },
            &client,
        )
        .await
        .unwrap();
        assert_eq!(handles(&mine), vec!["ios-team", "old-team"]);
    }

    #[tokio::test]
    async fn delete_removes_the_head_only_for_its_author() {
        let (creator, relay) = setup().await;
        run(create("ios-team", &[pk(1)]), &creator).await.unwrap();
        let url = fake_relay(relay.clone()).await;
        let editor = client_for(&url);
        run(
            GroupsCmd::AddMember {
                handle: "ios-team".into(),
                members: vec![pk(2)],
            },
            &editor,
        )
        .await
        .unwrap();

        let err = run(
            GroupsCmd::Delete {
                handle: "ios-team".into(),
            },
            &creator,
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("--disable"), "{err}");

        let deleted = run(
            GroupsCmd::Delete {
                handle: "ios-team".into(),
            },
            &editor,
        )
        .await
        .unwrap();
        assert_eq!(deleted["handle"], "ios-team");
        let missing = run(
            GroupsCmd::Get {
                handle: "ios-team".into(),
            },
            &creator,
        )
        .await
        .unwrap_err();
        assert!(matches!(missing, CliError::NotFound(_)), "{missing}");
    }

    #[tokio::test]
    async fn a_relay_refusal_is_an_error_with_its_message() {
        let (client, relay) = setup().await;
        run(create("ios-team", &[]), &client).await.unwrap();
        relay.lock().unwrap().reject = Some("invalid: only the group creator".into());
        let err = run(
            GroupsCmd::AddMember {
                handle: "ios-team".into(),
                members: vec![pk(1)],
            },
            &client,
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("only the group creator"), "{err}");
    }

    #[tokio::test]
    async fn missing_group_is_not_found() {
        let (client, _relay) = setup().await;
        for cmd in [
            GroupsCmd::Get {
                handle: "ghost".into(),
            },
            GroupsCmd::Members {
                handle: "ghost".into(),
            },
            GroupsCmd::AddMember {
                handle: "ghost".into(),
                members: vec![pk(1)],
            },
        ] {
            let err = run(cmd, &client).await.unwrap_err();
            assert!(matches!(err, CliError::NotFound(_)), "{err}");
        }
    }

    #[tokio::test]
    async fn mention_tags_mark_each_member_and_skip_explicit_mentions() {
        let (client, _relay) = setup().await;
        run(create("ios-team", &[pk(1), pk(2), pk(3)]), &client)
            .await
            .unwrap();
        let tags = mention_tags(&client, &["@ios-team".into()], &[pk(2)])
            .await
            .unwrap();
        let tags: Vec<Vec<String>> = tags.iter().map(|t| t.as_slice().to_vec()).collect();
        assert_eq!(
            tags,
            vec![
                vec!["p".to_string(), pk(1), "".into(), "group:ios-team".into()],
                vec!["p".to_string(), pk(3), "".into(), "group:ios-team".into()],
            ]
        );

        let six: Vec<String> = (0..=MAX_GROUP_MENTIONS_PER_EVENT)
            .map(|i| format!("g{i}"))
            .collect();
        let err = mention_tags(&client, &six, &[]).await.unwrap_err();
        assert!(matches!(err, CliError::Usage(_)), "{err}");

        run(create("empty", &[]), &client).await.unwrap();
        let err = mention_tags(&client, &["empty".into()], &[]).await;
        assert!(err.is_err(), "an empty group would notify no one");
    }
}
