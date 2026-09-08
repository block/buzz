use crate::{managed_agents::device_policy::model::DeviceAgentPolicy, models::SearchUsersResponse};

// A preferred first-page profile can displace a base result. Requery that
// same bounded page and drain its remaining results before advancing the relay
// cursor; clients treat this cursor as opaque.
pub(super) fn base_cursor(cursor: Option<&str>) -> Option<String> {
    cursor.map(|value| {
        if value.starts_with("preferred:") {
            "1".into()
        } else {
            value.into()
        }
    })
}

pub(super) async fn complete_search<F, Fut>(
    policy: &DeviceAgentPolicy,
    relay_url: &str,
    query: &str,
    limit: Option<u32>,
    cursor: Option<&str>,
    mut response: SearchUsersResponse,
    fetch: F,
) -> Result<SearchUsersResponse, String>
where
    F: FnOnce(Vec<String>) -> Fut,
    Fut: std::future::Future<Output = Result<Vec<nostr::Event>, String>>,
{
    let max = limit.unwrap_or(8).min(500) as usize;
    if max == 0 || nostr::PublicKey::parse(query.trim()).is_ok() {
        return Ok(response);
    }
    let first_page = cursor.and_then(|c| c.parse::<u32>().ok()).unwrap_or(1) <= 1;
    let normalized_query = query.trim().to_lowercase();
    let preferred: Vec<_> = policy
        .preferred_agents
        .iter()
        .filter(|p| {
            p.relay_url.trim_end_matches('/') == relay_url.trim_end_matches('/')
                && p.name.trim().to_lowercase().contains(&normalized_query)
        })
        .take(500)
        .collect();
    // Explicit author lookup prevents old identities from exhausting the name
    // search page before the preferred identity is even considered.
    if first_page && !preferred.is_empty() {
        let missing: Vec<String> = preferred
            .iter()
            .filter(|p| {
                !response.users.iter().any(|u| {
                    u.pubkey == p.pubkey
                        && u.owner_pubkey.as_deref() == Some(p.owner_pubkey.as_str())
                        && u.display_name
                            .as_deref()
                            .is_some_and(|n| n.trim().eq_ignore_ascii_case(p.name.trim()))
                })
            })
            .map(|p| p.pubkey.clone())
            .collect();
        if !missing.is_empty() {
            let events = fetch(missing).await?;
            let valid: Vec<_> = events
                .into_iter()
                .take(500)
                .filter(|event| event.kind == nostr::Kind::Metadata && event.verify().is_ok())
                .collect();
            for user in crate::nostr_convert::list_user_search_results(&valid, 500).users {
                if preferred.iter().any(|p| {
                    user.pubkey == p.pubkey
                        && user.owner_pubkey.as_deref() == Some(p.owner_pubkey.as_str())
                        && user
                            .display_name
                            .as_deref()
                            .is_some_and(|n| n.trim().eq_ignore_ascii_case(p.name.trim()))
                }) {
                    response
                        .users
                        .retain(|existing| existing.pubkey != user.pubkey);
                    response.users.push(user);
                }
            }
        }
        // Keep canonical results inside the caller's cap even when the base
        // page also contains unrelated people with matching names.
        response
            .users
            .sort_by_key(|user| !preferred.iter().any(|p| user.pubkey == p.pubkey));
    }
    response.users.retain(|user| {
        !user.is_agent
            || policy.allows_identity(
                relay_url,
                user.owner_pubkey.as_deref(),
                user.display_name.as_deref().unwrap_or(""),
                &user.pubkey,
            )
    });
    if !first_page {
        response
            .users
            .retain(|user| !preferred.iter().any(|p| user.pubkey == p.pubkey));
    }
    let offset = cursor
        .and_then(|c| c.strip_prefix("preferred:"))
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0)
        .min(1000);
    if first_page && response.users.len() > offset + max {
        response.next_cursor = Some(format!("preferred:{}", offset + max));
    }
    response.users = response.users.into_iter().skip(offset).take(max).collect();
    Ok(response)
}

#[cfg(test)]
#[path = "profile_preferred_search_tests.rs"]
mod tests;
