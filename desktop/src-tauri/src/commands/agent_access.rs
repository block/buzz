use crate::app_state::AppState;

/// NIP-11 `supported_extensions` identifier a relay advertises when it defers
/// to each agent's own published access policy. Mirrors
/// `buzz-relay`'s `AGENT_ACCESS_PUBLISHED_POLICY_EXTENSION`.
const AGENT_ACCESS_PUBLISHED_POLICY_EXTENSION: &str = "agent-access-published-policy";

/// Whether a NIP-11 document advertises the published-policy extension.
///
/// Split from the fetch so the parse is testable without a relay, and so a
/// malformed or partial document is a plain `false` rather than an error the
/// caller has to decide what to do with.
fn advertises_published_policy(document: &serde_json::Value) -> bool {
    document
        .get("supported_extensions")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|extensions| {
            extensions
                .iter()
                .filter_map(serde_json::Value::as_str)
                .any(|extension| extension == AGENT_ACCESS_PUBLISHED_POLICY_EXTENSION)
        })
}

/// Ask the workspace relay whether it governs agent access by published policy.
///
/// Fails closed: any transport, status, or parse problem answers `false`, which
/// leaves the build default in place. A relay that cannot be reached must never
/// widen what this client will show.
async fn relay_defers_to_published_policy(state: &AppState) -> bool {
    let url = crate::relay::relay_api_base_url_with_override(state);
    let Ok(response) = state
        .http_client
        .get(&url)
        .header("Accept", "application/nostr+json")
        .send()
        .await
    else {
        return false;
    };
    if !response.status().is_success() {
        return false;
    }
    match response.json::<serde_json::Value>().await {
        Ok(document) => advertises_published_policy(&document),
        Err(_) => false,
    }
}

/// Return whether this client should hide relay agents owned by someone else.
///
/// Two inputs, and the distinction matters:
///
/// - **The build.** A marked release compiles
///   `BUZZ_DESKTOP_BUILD_AGENT_ACCESS_OWNER_ONLY` and defaults to hiding them.
///   OSS builds do not and never did.
/// - **The relay.** A deployment that advertises
///   `agent-access-published-policy` in NIP-11 has declared that an agent's own
///   published, NIP-OA-attested `respond_to` decides who may mention it. On such
///   a relay a marked build defers to that, because the operator and the agent
///   owner are the ones who set the policy and carry the risk — and because a
///   relay's users must not each have to configure their own app for their
///   operator's decision to take effect.
///
/// Scope is deliberately narrow. This feeds the mention gate only. Local spawn
/// and provider deployment still clamp through
/// [`crate::managed_agents::owner_only`], so an agent this build *runs* is
/// unaffected and its harness-side guard stays pinned.
///
/// Never returns `Err`: the frontend treats an errored policy query as
/// "unknown" and hides every agent, so a relay hiccup must not empty the
/// mention list. Unreachable or silent relays yield the build default.
#[tauri::command]
pub async fn agent_access_owner_only(state: tauri::State<'_, AppState>) -> Result<bool, String> {
    if !crate::managed_agents::owner_only_access_build() {
        return Ok(false);
    }
    Ok(!relay_defers_to_published_policy(&state).await)
}

#[cfg(test)]
mod tests {
    use super::{advertises_published_policy, AGENT_ACCESS_PUBLISHED_POLICY_EXTENSION};
    use serde_json::json;

    #[test]
    #[ignore = "requires BUZZ_TEST_EXPECTED_AGENT_ACCESS_OWNER_ONLY"]
    fn compiled_policy_matches_expected() {
        let expected = std::env::var("BUZZ_TEST_EXPECTED_AGENT_ACCESS_OWNER_ONLY")
            .expect("BUZZ_TEST_EXPECTED_AGENT_ACCESS_OWNER_ONLY must be set")
            .parse::<bool>()
            .expect("BUZZ_TEST_EXPECTED_AGENT_ACCESS_OWNER_ONLY must be true or false");
        assert_eq!(
            crate::managed_agents::owner_only_access_build(),
            expected,
            "build flag drives the default; the relay only relaxes it"
        );
    }

    #[test]
    fn advertised_extension_is_recognised() {
        let document = json!({
            "supported_extensions": ["nip-er", AGENT_ACCESS_PUBLISHED_POLICY_EXTENSION]
        });
        assert!(advertises_published_policy(&document));
    }

    #[test]
    fn a_relay_that_stays_silent_keeps_the_build_default() {
        for document in [
            json!({}),
            json!({ "supported_extensions": [] }),
            json!({ "supported_extensions": ["nip-er", "nip-pl"] }),
            // Wrong shapes must not be read as consent.
            json!({ "supported_extensions": AGENT_ACCESS_PUBLISHED_POLICY_EXTENSION }),
            json!({ "supported_extensions": [{ "name": AGENT_ACCESS_PUBLISHED_POLICY_EXTENSION }] }),
        ] {
            assert!(
                !advertises_published_policy(&document),
                "unexpected opt-in for {document}"
            );
        }
    }

    #[test]
    fn extension_id_matches_the_relay_constant() {
        // Kept in sync by hand across crates; assert the literal so a rename on
        // either side fails here rather than silently disabling the feature.
        assert_eq!(
            AGENT_ACCESS_PUBLISHED_POLICY_EXTENSION,
            "agent-access-published-policy"
        );
    }
}
