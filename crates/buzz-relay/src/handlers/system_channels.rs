//! Query scope for system channels.
//!
//! A system channel (`channel_type = 'system'`) exists only for access
//! control. The implicit channel set of a query, from
//! `get_accessible_channel_ids`, leaves system channels out. A filter adds
//! the system channels that the reader can read (member, or open) only when
//! it names them:
//!
//! - `#h` lists the channel, on any kinds;
//! - `#d` lists the channel, on a filter whose kinds are all 39000–39003.
//!
//! The addition is per filter. Another filter in the same request keeps the
//! implicit set, so naming one system channel cannot widen a sibling filter.

use std::borrow::Cow;

use nostr::{Alphabet, Filter, SingleLetterTag};
use uuid::Uuid;

use buzz_core::CommunityId;

use crate::state::AppState;

/// Whether every kind in `filter` is a NIP-29 group-state kind (39000–39003).
pub(crate) fn filter_is_group_state_only(filter: &Filter) -> bool {
    filter.kinds.as_ref().is_some_and(|kinds| {
        !kinds.is_empty()
            && kinds.iter().all(|kind| {
                (buzz_core::kind::KIND_NIP29_GROUP_METADATA
                    ..=buzz_core::kind::KIND_NIP29_GROUP_ROLES)
                    .contains(&u32::from(kind.as_u16()))
            })
    })
}

/// Channel IDs that `filter` names: its `#h` values, and its `#d` values when
/// the filter asks only for group-state kinds. Malformed values are skipped.
fn named_channels(filter: &Filter) -> impl Iterator<Item = Uuid> + '_ {
    let h = filter
        .generic_tags
        .get(&SingleLetterTag::lowercase(Alphabet::H));
    let d = filter_is_group_state_only(filter)
        .then(|| {
            filter
                .generic_tags
                .get(&SingleLetterTag::lowercase(Alphabet::D))
        })
        .flatten();
    h.into_iter()
        .chain(d)
        .flatten()
        .filter_map(|value| value.parse::<Uuid>().ok())
}

/// Whether `filter` could need a system channel added to its scope.
fn filter_needs_system_channels(filter: &Filter, implicit: &[Uuid]) -> bool {
    named_channels(filter).any(|id| !implicit.contains(&id))
}

/// The channels `filter` may read: the implicit set, plus the readable
/// system channels the filter names.
pub(crate) fn filter_channel_scope<'a>(
    filter: &Filter,
    implicit: &'a [Uuid],
    readable_system: &[Uuid],
) -> Cow<'a, [Uuid]> {
    if readable_system.is_empty() {
        return Cow::Borrowed(implicit);
    }
    let mut extra: Vec<Uuid> = Vec::new();
    for id in named_channels(filter) {
        if readable_system.contains(&id) && !implicit.contains(&id) && !extra.contains(&id) {
            extra.push(id);
        }
    }
    if extra.is_empty() {
        return Cow::Borrowed(implicit);
    }
    let mut scope = implicit.to_vec();
    scope.extend(extra);
    Cow::Owned(scope)
}

/// Load the system channels the reader can read, narrowed to the token's
/// channel scope, when at least one filter could use them. Returns an empty
/// set, without a database read, when no filter names a channel outside the
/// implicit set.
pub(crate) async fn readable_system_channels_for_filters(
    state: &AppState,
    community: CommunityId,
    pubkey: &[u8],
    filters: &[Filter],
    implicit: &[Uuid],
    token_channel_ids: Option<&[Uuid]>,
) -> Result<Vec<Uuid>, buzz_db::DbError> {
    if !filters
        .iter()
        .any(|filter| filter_needs_system_channels(filter, implicit))
    {
        return Ok(Vec::new());
    }
    let mut readable = state
        .db
        .get_readable_system_channel_ids(community, pubkey)
        .await?;
    if let Some(allowed) = token_channel_ids {
        readable.retain(|id| allowed.contains(id));
    }
    Ok(readable)
}

/// Move system channels out of the repaired tail of the implicit set.
///
/// A request re-checks membership for named channels missing from its first
/// lookups, and [`super::req::resolve_request_local_access`] appends each
/// confirmed one to the implicit set. A system channel lands there when the
/// reader joins it between the lookups and the re-check. This moves it to
/// `readable_system`, so only the filters that name it can read it.
///
/// `repaired_from` is the length of `implicit` before the re-check. The tail
/// after it holds the repaired channels; nothing before it is a system
/// channel.
pub(crate) async fn keep_repaired_system_channels_named(
    state: &AppState,
    community: CommunityId,
    implicit: &mut Vec<Uuid>,
    repaired_from: usize,
    readable_system: &mut Vec<Uuid>,
) -> Result<(), buzz_db::DbError> {
    let Some(repaired) = implicit
        .get(repaired_from..)
        .filter(|tail| !tail.is_empty())
    else {
        return Ok(());
    };
    let system = state
        .db
        .system_channel_ids_among(community, repaired)
        .await?;
    move_system_channels(implicit, repaired_from, &system, readable_system);
    Ok(())
}

fn move_system_channels(
    implicit: &mut Vec<Uuid>,
    repaired_from: usize,
    system: &[Uuid],
    readable_system: &mut Vec<Uuid>,
) {
    if system.is_empty() {
        return;
    }
    let repaired = implicit.split_off(repaired_from);
    for id in repaired {
        if !system.contains(&id) {
            implicit.push(id);
        } else if !readable_system.contains(&id) {
            readable_system.push(id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filter(json: serde_json::Value) -> Filter {
        serde_json::from_value(json).expect("filter")
    }

    #[test]
    fn named_system_channel_widens_only_its_own_filter() {
        let open = Uuid::new_v4();
        let system = Uuid::new_v4();
        let implicit = vec![open];
        let readable = vec![system];

        let by_h = filter(serde_json::json!({"#h": [system.to_string()]}));
        assert_eq!(
            filter_channel_scope(&by_h, &implicit, &readable).as_ref(),
            &[open, system]
        );

        let by_d = filter(serde_json::json!({"kinds": [39000], "#d": [system.to_string()]}));
        assert_eq!(
            filter_channel_scope(&by_d, &implicit, &readable).as_ref(),
            &[open, system]
        );

        // #d names a channel only on group-state kinds.
        let d_other_kind =
            filter(serde_json::json!({"kinds": [39000, 1], "#d": [system.to_string()]}));
        assert_eq!(
            filter_channel_scope(&d_other_kind, &implicit, &readable).as_ref(),
            &[open]
        );

        // A sibling filter that names nothing keeps the implicit set.
        let sibling = filter(serde_json::json!({"kinds": [9]}));
        assert_eq!(
            filter_channel_scope(&sibling, &implicit, &readable).as_ref(),
            &[open]
        );

        // An unreadable system channel is never added.
        let unreadable = Uuid::new_v4();
        let other = filter(serde_json::json!({"#h": [unreadable.to_string()]}));
        assert_eq!(
            filter_channel_scope(&other, &implicit, &readable).as_ref(),
            &[open]
        );
    }

    #[test]
    fn a_repaired_system_channel_stays_in_the_filter_that_names_it() {
        let open = Uuid::new_v4();
        let repaired_open = Uuid::new_v4();
        let joined_system = Uuid::new_v4();
        let mut implicit = vec![open, repaired_open, joined_system];
        let mut readable = Vec::new();
        move_system_channels(&mut implicit, 1, &[joined_system], &mut readable);
        assert_eq!(implicit, [open, repaired_open]);
        assert_eq!(readable, [joined_system]);

        let names_it = filter(serde_json::json!({"#h": [joined_system.to_string()]}));
        assert_eq!(
            filter_channel_scope(&names_it, &implicit, &readable).as_ref(),
            &[open, repaired_open, joined_system]
        );
        let sibling = filter(serde_json::json!({"kinds": [39000]}));
        assert_eq!(
            filter_channel_scope(&sibling, &implicit, &readable).as_ref(),
            &[open, repaired_open]
        );
    }

    #[test]
    fn lookup_runs_only_when_a_filter_names_a_channel_outside_the_implicit_set() {
        let open = Uuid::new_v4();
        let implicit = vec![open];
        assert!(!filter_needs_system_channels(
            &filter(serde_json::json!({"kinds": [9]})),
            &implicit
        ));
        assert!(!filter_needs_system_channels(
            &filter(serde_json::json!({"#h": [open.to_string()]})),
            &implicit
        ));
        assert!(filter_needs_system_channels(
            &filter(serde_json::json!({"#h": [Uuid::new_v4().to_string()]})),
            &implicit
        ));
        assert!(filter_needs_system_channels(
            &filter(serde_json::json!({"kinds": [39002], "#d": [Uuid::new_v4().to_string()]})),
            &implicit
        ));
        assert!(!filter_needs_system_channels(
            &filter(serde_json::json!({"kinds": [1], "#d": [Uuid::new_v4().to_string()]})),
            &implicit
        ));
    }
}
