//! Link and unlink existing channels from Projects through kind:47010 commands.

use std::collections::BTreeSet;

use buzz_core::kind::{KIND_PROJECT, KIND_PROJECT_RELATED_CHANNELS_SNAPSHOT};
use buzz_sdk::{
    build_project_related_channel_command, parse_project_related_channels_snapshot,
    ProjectRelatedChannelCoordinate, ProjectRelatedChannelOperation,
    PROJECT_RELATED_CHANNELS_SNAPSHOT_CAP,
};
use nostr::Event;
use uuid::Uuid;

use crate::client::{normalize_write_response, BuzzClient};
use crate::error::CliError;

fn sdk_error(error: buzz_sdk::SdkError) -> CliError {
    CliError::Usage(error.to_string())
}

async fn submit_related_channel_command(
    client: &BuzzClient,
    project_coordinate: &ProjectRelatedChannelCoordinate,
    channel_id: Uuid,
    operation: ProjectRelatedChannelOperation,
) -> Result<String, CliError> {
    let builder = build_project_related_channel_command(
        &project_coordinate.coordinate,
        channel_id,
        operation,
    )
    .map_err(sdk_error)?;
    let event = client.sign_event(builder)?;
    client
        .submit_event(event)
        .await
        .map(|raw| normalize_write_response(&raw))
}

/// Link an existing channel to a Project.
pub async fn cmd_link_channel(
    client: &BuzzClient,
    project: &str,
    channel: &str,
) -> Result<(), CliError> {
    let project = ProjectRelatedChannelCoordinate::parse(project).map_err(sdk_error)?;
    let channel =
        Uuid::parse_str(channel).map_err(|_| CliError::Usage("channel must be a UUID".into()))?;
    let response = submit_related_channel_command(
        client,
        &project,
        channel,
        ProjectRelatedChannelOperation::Add,
    )
    .await?;
    println!("{response}");
    Ok(())
}

/// Unlink an existing channel from a Project.
pub async fn cmd_unlink_channel(
    client: &BuzzClient,
    project: &str,
    channel: &str,
) -> Result<(), CliError> {
    let project = ProjectRelatedChannelCoordinate::parse(project).map_err(sdk_error)?;
    let channel =
        Uuid::parse_str(channel).map_err(|_| CliError::Usage("channel must be a UUID".into()))?;
    let response = submit_related_channel_command(
        client,
        &project,
        channel,
        ProjectRelatedChannelOperation::Remove,
    )
    .await?;
    println!("{response}");
    Ok(())
}

fn normalize_relay_self_hex(value: &str) -> Result<String, CliError> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(CliError::Other(
            "relay info document has an invalid `self` pubkey".into(),
        ));
    }
    Ok(value.to_ascii_lowercase())
}

fn verify_related_channels_snapshot(
    event: &Event,
    relay_self: &str,
    project_coordinate: &str,
    project_event_id: &str,
) -> Result<Vec<Uuid>, CliError> {
    if event.pubkey.to_hex() != relay_self {
        return Err(CliError::Other(
            "Project related-channel snapshot author does not match relay self".into(),
        ));
    }
    event.verify().map_err(|error| {
        CliError::Other(format!(
            "Project related-channel snapshot signature is invalid: {error}"
        ))
    })?;
    parse_project_related_channels_snapshot(event, project_coordinate, project_event_id).map_err(
        |error| {
            CliError::Other(format!(
                "Project related-channel snapshot is invalid: {error}"
            ))
        },
    )
}

fn canonical_channel_uuid(value: &str) -> Option<Uuid> {
    let channel = Uuid::parse_str(value).ok()?;
    if channel.is_nil() || !value.eq_ignore_ascii_case(&channel.to_string()) {
        return None;
    }
    Some(channel)
}

fn legacy_related_channels(
    event: &Event,
    project: &ProjectRelatedChannelCoordinate,
) -> Result<Vec<Uuid>, CliError> {
    if u32::from(event.kind.as_u16()) != KIND_PROJECT || event.pubkey != project.owner {
        return Err(CliError::Other(
            "legacy Project related-channel fallback returned the wrong event".into(),
        ));
    }
    event.verify().map_err(|error| {
        CliError::Other(format!(
            "legacy Project related-channel fallback has an invalid signature: {error}"
        ))
    })?;
    let d_tags: Vec<_> = event
        .tags
        .iter()
        .filter(|tag| tag.as_slice().first().map(String::as_str) == Some("d"))
        .collect();
    if d_tags.len() != 1 || d_tags[0].as_slice() != ["d", project.project_d.as_str()] {
        return Err(CliError::Other(
            "legacy Project related-channel fallback has an invalid d tag".into(),
        ));
    }

    let home_channel = event
        .tags
        .iter()
        .find(|tag| tag.as_slice().first().map(String::as_str) == Some("buzz-channel"))
        .and_then(|tag| tag.as_slice().get(1))
        .and_then(|value| canonical_channel_uuid(value));
    let mut channels = BTreeSet::new();
    for tag in event.tags.iter() {
        let parts = tag.as_slice();
        if parts.first().map(String::as_str) != Some("buzz-related-channel") {
            continue;
        }
        let Some(channel) = parts
            .get(1)
            .filter(|_| parts.len() == 2)
            .and_then(|value| canonical_channel_uuid(value))
        else {
            continue;
        };
        if Some(channel) != home_channel {
            channels.insert(channel);
        }
    }
    Ok(channels
        .into_iter()
        .take(PROJECT_RELATED_CHANNELS_SNAPSHOT_CAP)
        .collect())
}

async fn fetch_current_project(
    client: &BuzzClient,
    project: &ProjectRelatedChannelCoordinate,
) -> Result<(Event, Vec<Uuid>), CliError> {
    let filter = serde_json::json!({
        "kinds": [KIND_PROJECT],
        "authors": [project.owner.to_hex()],
        "#d": [project.project_d],
        "limit": 2,
    });
    let raw = client.query(&filter).await?;
    let events: Vec<Event> = serde_json::from_str(&raw)
        .map_err(|error| CliError::Other(format!("invalid Project query response: {error}")))?;
    match events.as_slice() {
        [] => Err(CliError::Other("Project not found".into())),
        [event] => {
            let channels = legacy_related_channels(event, project)?;
            Ok((event.clone(), channels))
        }
        _ => Err(CliError::Other(
            "relay returned multiple current Project events".into(),
        )),
    }
}

async fn fetch_related_channels(
    client: &BuzzClient,
    project: &str,
) -> Result<serde_json::Value, CliError> {
    let project = ProjectRelatedChannelCoordinate::parse(project).map_err(sdk_error)?;
    let info_raw = client
        .get_public("/")
        .await
        .map_err(|error| CliError::Other(format!("failed to fetch relay info: {error}")))?;
    let info: serde_json::Value = serde_json::from_str(&info_raw)
        .map_err(|error| CliError::Other(format!("relay info is not valid JSON: {error}")))?;
    let relay_self = info
        .get("self")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| CliError::Other("relay info document is missing `self`".into()))?;
    let relay_self = normalize_relay_self_hex(relay_self)?;
    let (project_event, legacy_channels) = fetch_current_project(client, &project).await?;
    let snapshot_d = buzz_core::project_related_channels::project_related_channels_snapshot_d(
        &project.coordinate,
    );
    let filter = serde_json::json!({
        "kinds": [KIND_PROJECT_RELATED_CHANNELS_SNAPSHOT],
        "authors": [relay_self],
        "#d": [snapshot_d],
        "limit": 2,
    });
    let raw = client.query(&filter).await?;
    let events: Vec<Event> = serde_json::from_str(&raw)
        .map_err(|error| CliError::Other(format!("invalid snapshot query response: {error}")))?;
    if events.len() > 1 {
        return Err(CliError::Other(
            "relay returned multiple Project related-channel snapshots".into(),
        ));
    }
    let Some(event) = events.first() else {
        return Ok(serde_json::json!({
            "project": project.coordinate,
            "snapshot_found": false,
            "source": "project_metadata",
            "related_channels": legacy_channels.iter().map(Uuid::to_string).collect::<Vec<_>>(),
        }));
    };
    let channels = verify_related_channels_snapshot(
        event,
        &relay_self,
        &project.coordinate,
        &project_event.id.to_hex(),
    )?;
    Ok(serde_json::json!({
        "project": project.coordinate,
        "snapshot_found": true,
        "source": "relay_snapshot",
        "related_channels": channels.iter().map(Uuid::to_string).collect::<Vec<_>>(),
    }))
}

/// Read the relay's authoritative related-channel projection for one Project.
pub async fn cmd_related_channels(client: &BuzzClient, project: &str) -> Result<(), CliError> {
    println!("{}", fetch_related_channels(client, project).await?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use buzz_sdk::{
        build_project_related_channels_snapshot, parse_project_related_channel_command,
    };
    use nostr::{Event, EventBuilder, Keys, Kind, Tag};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    use super::*;

    const CHANNEL: &str = "11111111-1111-4111-8111-111111111111";

    fn accepted(message: &str) -> String {
        serde_json::json!({
            "event_id": "a".repeat(64),
            "accepted": true,
            "message": message,
            "ignored": true,
        })
        .to_string()
    }

    async fn run_with_responses(
        operation: ProjectRelatedChannelOperation,
        responses: Vec<(u16, String)>,
    ) -> (Result<String, CliError>, Vec<Event>) {
        let actor = Keys::generate();
        let coordinate = ProjectRelatedChannelCoordinate::parse(&format!(
            "30621:{}:project",
            Keys::generate().public_key().to_hex()
        ))
        .unwrap();
        let channel = Uuid::parse_str(CHANNEL).unwrap();
        let posted_events = Arc::new(Mutex::new(Vec::new()));
        let captured_events = posted_events.clone();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            for (status, response_body) in responses {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut buffer = vec![0; 65_536];
                let read = socket.read(&mut buffer).await.unwrap();
                let request = String::from_utf8_lossy(&buffer[..read]);
                let body = request.split("\r\n\r\n").nth(1).unwrap();
                let event: Event = serde_json::from_str(body).unwrap();
                captured_events.lock().unwrap().push(event);
                let reason = if status == 200 { "OK" } else { "Bad Request" };
                let response = format!(
                    "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response_body}",
                    response_body.len()
                );
                socket.write_all(response.as_bytes()).await.unwrap();
            }
        });
        let auth = Tag::parse(["auth", &"a".repeat(64), "", &"b".repeat(128)]).unwrap();
        let auth_json = serde_json::to_string(&auth).unwrap();
        let client = BuzzClient::new(base_url, actor, Some(auth), Some(auth_json)).unwrap();

        let result = submit_related_channel_command(&client, &coordinate, channel, operation).await;
        server.await.unwrap();
        let events = posted_events.lock().unwrap().clone();
        (result, events)
    }

    #[tokio::test]
    async fn initial_apply_and_noop_each_use_one_direct_submission() {
        for message in ["", "no-op: related channel already has requested state"] {
            let (result, events) = run_with_responses(
                ProjectRelatedChannelOperation::Add,
                vec![(200, accepted(message))],
            )
            .await;
            let response: serde_json::Value =
                serde_json::from_str(&result.expect("accepted response")).unwrap();
            assert_eq!(response["accepted"], true);
            assert_eq!(response["message"], message);
            assert!(response.get("ignored").is_none());
            assert_eq!(events.len(), 1);
            let parsed = parse_project_related_channel_command(&events[0]).unwrap();
            assert_eq!(parsed.operation, ProjectRelatedChannelOperation::Add);
            assert_eq!(
                events[0]
                    .tags
                    .iter()
                    .filter(|tag| tag.as_slice().first().map(String::as_str) == Some("auth"))
                    .count(),
                1
            );
            let names: Vec<&str> = events[0]
                .tags
                .iter()
                .map(|tag| tag.as_slice()[0].as_str())
                .collect();
            assert_eq!(names, ["a", "op", "d", "auth"]);
        }
    }

    #[tokio::test]
    async fn a_rejected_command_is_not_retried() {
        let (result, events) = run_with_responses(
            ProjectRelatedChannelOperation::Remove,
            vec![(
                400,
                serde_json::json!({ "error": "invalid: bad target" }).to_string(),
            )],
        )
        .await;

        assert!(matches!(result, Err(CliError::Relay { status: 400, .. })));
        assert_eq!(events.len(), 1);
        assert_eq!(
            parse_project_related_channel_command(&events[0])
                .unwrap()
                .operation,
            ProjectRelatedChannelOperation::Remove
        );
    }

    #[test]
    fn snapshot_verification_requires_relay_author_and_valid_signature() {
        let relay = Keys::generate();
        let project = format!("30621:{}:project", Keys::generate().public_key().to_hex());
        let channel = Uuid::parse_str(CHANNEL).unwrap();
        let project_event_id = "11".repeat(32);
        let snapshot =
            build_project_related_channels_snapshot(&project, &project_event_id, [channel], 1)
                .unwrap()
                .sign_with_keys(&relay)
                .unwrap();
        assert_eq!(
            verify_related_channels_snapshot(
                &snapshot,
                &relay.public_key().to_hex(),
                &project,
                &project_event_id,
            )
            .unwrap(),
            [channel]
        );
        assert!(verify_related_channels_snapshot(
            &snapshot,
            &Keys::generate().public_key().to_hex(),
            &project,
            &project_event_id,
        )
        .is_err());
        assert!(verify_related_channels_snapshot(
            &snapshot,
            &relay.public_key().to_hex(),
            &project,
            &"22".repeat(32),
        )
        .is_err());
    }

    #[test]
    fn legacy_fallback_deduplicates_sorts_and_bounds_channels() {
        let owner = Keys::generate();
        let project = ProjectRelatedChannelCoordinate::parse(&format!(
            "30621:{}:project",
            owner.public_key().to_hex()
        ))
        .unwrap();
        let mut tags = vec![Tag::parse(["d", "project"]).unwrap()];
        for index in (1_u64..=65).rev() {
            let channel = format!("11111111-1111-4111-8111-{index:012x}");
            tags.push(Tag::parse(["buzz-related-channel", &channel]).unwrap());
            tags.push(Tag::parse(["buzz-related-channel", &channel]).unwrap());
        }
        let event = EventBuilder::new(Kind::Custom(KIND_PROJECT as u16), "")
            .tags(tags)
            .sign_with_keys(&owner)
            .unwrap();

        let channels = legacy_related_channels(&event, &project).unwrap();
        assert_eq!(channels.len(), PROJECT_RELATED_CHANNELS_SNAPSHOT_CAP);
        assert!(channels.windows(2).all(|pair| pair[0] < pair[1]));
        assert_eq!(
            channels[0],
            Uuid::parse_str("11111111-1111-4111-8111-000000000001").unwrap()
        );
    }

    async fn run_snapshot_read(
        snapshot: Option<Event>,
        project_event: Option<Event>,
        relay: &Keys,
        project: &str,
    ) -> (Result<serde_json::Value, CliError>, Vec<serde_json::Value>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let relay_self = relay.public_key().to_hex();
        let has_project = project_event.is_some();
        let mut bodies = vec![
            serde_json::json!({ "self": relay_self }).to_string(),
            serde_json::to_string(&project_event.into_iter().collect::<Vec<_>>()).unwrap(),
        ];
        if has_project {
            bodies.push(serde_json::to_string(&snapshot.into_iter().collect::<Vec<_>>()).unwrap());
        }
        let captured_filters = Arc::new(Mutex::new(Vec::new()));
        let captured = captured_filters.clone();
        let server = tokio::spawn(async move {
            for body in bodies {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut buffer = vec![0; 65_536];
                let read = socket.read(&mut buffer).await.unwrap();
                let request = String::from_utf8_lossy(&buffer[..read]);
                if request.starts_with("POST /query ") {
                    let request_body = request.split("\r\n\r\n").nth(1).unwrap();
                    captured
                        .lock()
                        .unwrap()
                        .push(serde_json::from_str(request_body).unwrap());
                }
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                socket.write_all(response.as_bytes()).await.unwrap();
            }
        });
        let client = BuzzClient::new(base_url, Keys::generate(), None, None).unwrap();
        let result = fetch_related_channels(&client, project).await;
        server.await.unwrap();
        let filters = captured_filters.lock().unwrap().clone();
        (result, filters)
    }

    #[tokio::test]
    async fn related_channels_reads_trusted_snapshot_by_d_and_relay_author() {
        let relay = Keys::generate();
        let owner = Keys::generate();
        let project = format!("30621:{}:project", owner.public_key().to_hex());
        let project_event = EventBuilder::new(Kind::Custom(KIND_PROJECT as u16), "")
            .tag(Tag::parse(["d", "project"]).unwrap())
            .sign_with_keys(&owner)
            .unwrap();
        let channel = Uuid::parse_str(CHANNEL).unwrap();
        let snapshot = build_project_related_channels_snapshot(
            &project,
            &project_event.id.to_hex(),
            [channel],
            1,
        )
        .unwrap()
        .sign_with_keys(&relay)
        .unwrap();

        let (result, filters) =
            run_snapshot_read(Some(snapshot), Some(project_event), &relay, &project).await;
        let value = result.unwrap();
        assert_eq!(value["snapshot_found"], true);
        assert_eq!(value["source"], "relay_snapshot");
        assert_eq!(value["related_channels"], serde_json::json!([CHANNEL]));
        assert_eq!(filters.len(), 2);
        assert_eq!(filters[0][0]["kinds"], serde_json::json!([KIND_PROJECT]));
        let filter = &filters[1];
        assert_eq!(filter[0]["kinds"], serde_json::json!([30623]));
        assert_eq!(
            filter[0]["authors"],
            serde_json::json!([relay.public_key().to_hex()])
        );
        assert_eq!(
            filter[0]["#d"],
            serde_json::json!([
                buzz_core::project_related_channels::project_related_channels_snapshot_d(&project)
            ])
        );
    }

    #[tokio::test]
    async fn related_channels_falls_back_to_scoped_legacy_project_metadata() {
        let relay = Keys::generate();
        let owner = Keys::generate();
        let project = format!("30621:{}:project", owner.public_key().to_hex());
        let home = Uuid::parse_str("22222222-2222-4222-8222-222222222222").unwrap();
        let related = Uuid::parse_str(CHANNEL).unwrap();
        let project_event = EventBuilder::new(Kind::Custom(KIND_PROJECT as u16), "")
            .tags([
                Tag::parse(["d", "project"]).unwrap(),
                Tag::parse(["buzz-channel", &home.to_string().to_uppercase()]).unwrap(),
                Tag::parse(["buzz-related-channel", &home.to_string()]).unwrap(),
                Tag::parse(["buzz-related-channel", &related.to_string().to_uppercase()]).unwrap(),
                Tag::parse(["buzz-related-channel", &related.to_string()]).unwrap(),
                Tag::parse(["buzz-related-channel", "not-a-uuid"]).unwrap(),
            ])
            .sign_with_keys(&owner)
            .unwrap();
        let (result, filters) =
            run_snapshot_read(None, Some(project_event), &relay, &project).await;
        let value = result.unwrap();
        assert_eq!(value["snapshot_found"], false);
        assert_eq!(value["source"], "project_metadata");
        assert_eq!(value["related_channels"], serde_json::json!([CHANNEL]));
        assert_eq!(filters.len(), 2);
        assert_eq!(filters[0][0]["kinds"], serde_json::json!([KIND_PROJECT]));
        assert_eq!(
            filters[0][0]["authors"],
            serde_json::json!([owner.public_key().to_hex()])
        );
        assert_eq!(filters[0][0]["#d"], serde_json::json!(["project"]));
        assert_eq!(filters[0][0]["limit"], 2);
        assert_eq!(filters[1][0]["kinds"], serde_json::json!([30623]));
    }

    #[tokio::test]
    async fn related_channels_fail_closed_without_a_live_project() {
        let relay = Keys::generate();
        let owner = Keys::generate();
        let project = format!("30621:{}:project", owner.public_key().to_hex());
        let (result, filters) = run_snapshot_read(None, None, &relay, &project).await;

        assert!(matches!(result, Err(CliError::Other(message)) if message == "Project not found"));
        assert_eq!(
            filters.len(),
            1,
            "snapshot must not be read without a live Project"
        );
    }

    #[tokio::test]
    async fn related_channels_reject_a_snapshot_for_an_old_project_head() {
        let relay = Keys::generate();
        let owner = Keys::generate();
        let project = format!("30621:{}:project", owner.public_key().to_hex());
        let live_project = EventBuilder::new(Kind::Custom(KIND_PROJECT as u16), "")
            .tag(Tag::parse(["d", "project"]).unwrap())
            .sign_with_keys(&owner)
            .unwrap();
        let snapshot = build_project_related_channels_snapshot(
            &project,
            &"11".repeat(32),
            [Uuid::parse_str(CHANNEL).unwrap()],
            1,
        )
        .unwrap()
        .sign_with_keys(&relay)
        .unwrap();

        let (result, _) =
            run_snapshot_read(Some(snapshot), Some(live_project), &relay, &project).await;
        assert!(matches!(
            result,
            Err(CliError::Other(message))
                if message.contains("does not match the current Project head")
        ));
    }
}
