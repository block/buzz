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
//! A filter whose kinds are all 39000–39003 also opts in to every readable
//! system channel when it has `#t:["system"]` (the channel type tag) or any
//! `#P` (the creator tag). The filter then matches as usual, so
//! `{kinds:[39000], #P:[creator], #t:[label]}` finds one creator's labeled
//! channel, system or not. Existing clients send neither on these kinds.
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

/// Whether `filter` opts in to every readable system channel: its kinds are
/// all group-state kinds, and it has `#t:["system"]` or any `#P`.
fn filter_opts_in(filter: &Filter) -> bool {
    if !filter_is_group_state_only(filter) {
        return false;
    }
    let system_type = filter
        .generic_tags
        .get(&SingleLetterTag::lowercase(Alphabet::T))
        .is_some_and(|values| {
            values
                .iter()
                .any(|value| value == buzz_core::channel::ChannelType::System.as_str())
        });
    let creator = filter
        .generic_tags
        .get(&SingleLetterTag::uppercase(Alphabet::P))
        .is_some_and(|values| !values.is_empty());
    system_type || creator
}

/// Whether `filter` could need a system channel added to its scope.
fn filter_needs_system_channels(filter: &Filter, implicit: &[Uuid]) -> bool {
    filter_opts_in(filter) || named_channels(filter).any(|id| !implicit.contains(&id))
}

/// The channels `filter` may read: the implicit set, plus the readable
/// system channels the filter names, or all of them when it opts in.
pub(crate) fn filter_channel_scope<'a>(
    filter: &Filter,
    implicit: &'a [Uuid],
    readable_system: &[Uuid],
) -> Cow<'a, [Uuid]> {
    if readable_system.is_empty() {
        return Cow::Borrowed(implicit);
    }
    let opted_in = filter_opts_in(filter);
    let mut extra: Vec<Uuid> = Vec::new();
    let candidates: Vec<Uuid> = if opted_in {
        readable_system.to_vec()
    } else {
        named_channels(filter).collect()
    };
    for id in candidates {
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
/// set, without a database read, when no filter opts in or names a channel
/// outside the implicit set.
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

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::Keys;

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
    fn system_type_or_creator_on_group_state_kinds_opts_in() {
        let open = Uuid::new_v4();
        let system = vec![Uuid::new_v4(), Uuid::new_v4()];
        let implicit = vec![open];
        let all = [vec![open], system.clone()].concat();
        let creator = Keys::generate().public_key().to_hex();

        for opt_in in [
            serde_json::json!({"kinds": [39000], "#t": ["system"]}),
            serde_json::json!({"kinds": [39002], "#p": [creator], "#t": ["system", "x"]}),
            serde_json::json!({"kinds": [39000, 39003], "#P": [creator]}),
            serde_json::json!({"kinds": [39000], "#P": [creator], "#t": ["agent-attention"]}),
        ] {
            let f = filter(opt_in.clone());
            assert!(filter_needs_system_channels(&f, &implicit), "{opt_in}");
            assert_eq!(
                filter_channel_scope(&f, &implicit, &system).as_ref(),
                all.as_slice(),
                "{opt_in}"
            );
        }
        for no_opt_in in [
            // Labels alone, or other kinds, do not opt in.
            serde_json::json!({"kinds": [39000], "#t": ["agent-attention"]}),
            serde_json::json!({"kinds": [39000, 9], "#t": ["system"]}),
            serde_json::json!({"kinds": [9], "#P": [creator]}),
            serde_json::json!({"#t": ["system"]}),
        ] {
            let f = filter(no_opt_in.clone());
            assert!(!filter_needs_system_channels(&f, &implicit), "{no_opt_in}");
            assert_eq!(
                filter_channel_scope(&f, &implicit, &system).as_ref(),
                &[open],
                "{no_opt_in}"
            );
        }
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
