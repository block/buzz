use super::*;
use crate::{managed_agents::device_policy::model::PreferredAgent, nostr_convert};
use nostr::{Event, EventBuilder, Keys, Kind, Tag};

fn profile(owner: &Keys, agent: &Keys, name: &str) -> Event {
    let auth = buzz_sdk_pkg::nip_oa::compute_auth_tag(owner, &agent.public_key(), "").unwrap();
    let values: Vec<String> = serde_json::from_str(&auth).unwrap();
    EventBuilder::new(
        Kind::Metadata,
        serde_json::json!({"name": name}).to_string(),
    )
    .tags([Tag::parse(values).unwrap()])
    .sign_with_keys(agent)
    .unwrap()
}

fn fixture() -> (DeviceAgentPolicy, Event, SearchUsersResponse) {
    let owner = Keys::generate();
    let agent = Keys::generate();
    let preferred = profile(&owner, &agent, "Scout");
    let policy = DeviceAgentPolicy {
        preferred_agents: vec![PreferredAgent {
            relay_url: "https://relay.example".into(),
            owner_pubkey: owner.public_key().to_hex(),
            name: "Scout".into(),
            pubkey: agent.public_key().to_hex(),
            persona_id: None,
        }],
        ..Default::default()
    };
    let duplicates = (0..8)
        .map(|_| profile(&owner, &Keys::generate(), "Scout"))
        .collect::<Vec<_>>();
    let mut response = nostr_convert::list_user_search_results(&duplicates, 8);
    response.next_cursor = Some("2".into());
    (policy, preferred, response)
}

#[tokio::test]
async fn preferred_identity_outside_first_page_is_fetched_before_filtering() {
    let (policy, preferred, response) = fixture();
    let key = preferred.pubkey.to_hex();
    let expected = key.clone();
    let result = complete_search(
        &policy,
        "https://relay.example",
        "Sco",
        Some(8),
        None,
        response,
        |authors| async move {
            assert_eq!(authors, vec![expected]);
            Ok(vec![preferred])
        },
    )
    .await
    .unwrap();
    assert_eq!(result.users.len(), 1);
    assert_eq!(result.users[0].pubkey, key);
    assert_eq!(result.next_cursor.as_deref(), Some("2"));
}

#[tokio::test]
async fn failed_preferred_lookup_is_not_reported_as_an_empty_directory() {
    let (policy, _, response) = fixture();
    let result = complete_search(
        &policy,
        "https://relay.example",
        "Scout",
        None,
        None,
        response,
        |_| async { Err("offline".into()) },
    )
    .await;
    assert!(matches!(result, Err(error) if error.contains("offline")));
}

#[tokio::test]
async fn explicit_keys_and_other_communities_do_not_fetch_or_filter() {
    for (relay, query) in [
        ("https://other.example", "Scout".to_string()),
        (
            "https://relay.example",
            Keys::generate().public_key().to_hex(),
        ),
    ] {
        let (policy, _, response) = fixture();
        let result = complete_search(&policy, relay, &query, None, None, response, |_| async {
            panic!("unexpected preferred query")
        })
        .await
        .unwrap();
        assert_eq!(result.users.len(), 8);
    }
}

#[tokio::test]
async fn wrong_owner_and_tampered_profiles_cannot_supply_the_preference() {
    for tampered in [false, true] {
        let (mut policy, mut preferred, mut response) = fixture();
        if tampered {
            preferred.content = "{\"name\":\"Other\"}".into();
        } else {
            policy.preferred_agents[0].owner_pubkey = Keys::generate().public_key().to_hex();
            response.users.clear();
        }
        let result = complete_search(
            &policy,
            "https://relay.example",
            "Scout",
            None,
            None,
            response,
            |_| async { Ok(vec![preferred]) },
        )
        .await
        .unwrap();
        assert!(result.users.is_empty());
    }
}

#[tokio::test]
async fn preferred_result_stays_inside_limit_and_is_not_duplicated() {
    let (policy, preferred, mut response) = fixture();
    let key = preferred.pubkey.to_hex();
    response.users = vec![nostr_convert::user_search_result_from_event(&profile(
        &Keys::generate(),
        &Keys::generate(),
        "Scout",
    ))];
    let result = complete_search(
        &policy,
        "https://relay.example",
        "Scout",
        Some(1),
        None,
        response,
        |_| async { Ok(vec![preferred.clone(), preferred]) },
    )
    .await
    .unwrap();
    assert_eq!(result.users.len(), 1);
    assert_eq!(result.users[0].pubkey, key);
}

#[tokio::test]
async fn zero_limit_and_later_pages_do_not_fetch() {
    for (limit, cursor) in [(Some(0), None), (None, Some("2"))] {
        let (policy, _, mut response) = fixture();
        if limit == Some(0) {
            response.users.clear();
        }
        let result = complete_search(
            &policy,
            "https://relay.example",
            "Scout",
            limit,
            cursor,
            response,
            |_| async { panic!("unexpected preferred query") },
        )
        .await
        .unwrap();
        assert!(result.users.is_empty());
    }
}

#[tokio::test]
async fn pagination_preserves_displaced_people_and_does_not_repeat_preferred() {
    let (policy, preferred, _) = fixture();
    let unrelated = profile(&Keys::generate(), &Keys::generate(), "Scout");
    let page = || {
        let mut response =
            nostr_convert::list_user_search_results(std::slice::from_ref(&unrelated), 1);
        response.next_cursor = Some("2".into());
        response
    };
    let first = complete_search(
        &policy,
        "https://relay.example",
        "Scout",
        Some(1),
        None,
        page(),
        |_| async { Ok(vec![preferred.clone()]) },
    )
    .await
    .unwrap();
    assert_eq!(first.users[0].pubkey, preferred.pubkey.to_hex());
    assert_eq!(
        base_cursor(first.next_cursor.as_deref()).as_deref(),
        Some("1")
    );
    let next = complete_search(
        &policy,
        "https://relay.example",
        "Scout",
        Some(1),
        first.next_cursor.as_deref(),
        page(),
        |_| async { Ok(vec![preferred.clone()]) },
    )
    .await
    .unwrap();
    assert_eq!(next.users[0].pubkey, unrelated.pubkey.to_hex());
    assert_eq!(next.next_cursor.as_deref(), Some("2"));
    let later = complete_search(
        &policy,
        "https://relay.example",
        "Scout",
        Some(1),
        Some("2"),
        nostr_convert::list_user_search_results(&[preferred], 1),
        |_| async { panic!("unexpected query") },
    )
    .await
    .unwrap();
    assert!(later.users.is_empty());
}
