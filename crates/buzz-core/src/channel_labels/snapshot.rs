use nostr::{Event, PublicKey};
use uuid::Uuid;

use super::{validate_value, LabelError, LabelSet, MAX_LABELS, NAMESPACE};
use crate::channel::ChannelType;

/// Verify the complete NIP-CL snapshot against an explicitly trusted relay key.
///
/// The caller obtains `relay` from authenticated NIP-11 `self` or trusted
/// configuration, never from the snapshot. Scope caches by community and relay,
/// and use NIP-01 timestamp/lowest-ID ordering before replacing cached state.
/// CPU-bound signature verification belongs on a blocking thread in async code.
/// Invalid input is an error, never an authoritative empty label set.
pub fn verify_snapshot(
    event: &Event,
    relay: PublicKey,
    expected_channel: Uuid,
) -> Result<LabelSet, LabelError> {
    if crate::kind::event_kind_u32(event) != crate::kind::KIND_NIP29_GROUP_METADATA
        || event.pubkey != relay
        || expected_channel.is_nil()
        || crate::verify_event(event).is_err()
    {
        return Err(LabelError("untrusted channel metadata snapshot"));
    }
    let mut identifiers = Vec::new();
    let mut types = Vec::new();
    let mut namespace_count = 0;
    let mut values = Vec::new();
    for tag in event.tags.iter() {
        let parts = tag.as_slice();
        match parts[0].as_str() {
            "d" => identifiers.push(parts),
            "t" => types.push(parts),
            "label" => return Err(LabelError("legacy labels are not canonical snapshots")),
            "L" => {
                if parts.len() != 2 || parts[1] != NAMESPACE {
                    return Err(LabelError("invalid label namespace"));
                }
                namespace_count += 1;
            }
            "l" => {
                if parts.len() != 3 || parts[2] != NAMESPACE {
                    return Err(LabelError("invalid namespaced label"));
                }
                validate_value(&parts[1])?;
                if values.last().is_some_and(|previous| previous >= &parts[1]) {
                    return Err(LabelError("snapshot labels must be sorted and unique"));
                }
                values.push(parts[1].clone());
                if values.len() > MAX_LABELS {
                    return Err(LabelError("too many stored labels"));
                }
            }
            _ => {}
        }
    }
    let expected_id = expected_channel.to_string();
    if !matches!(identifiers.as_slice(), [parts] if parts.len() == 2 && parts[1] == expected_id) {
        return Err(LabelError(
            "snapshot does not identify the expected channel",
        ));
    }
    let channel_type: ChannelType = match types.as_slice() {
        [parts] if parts.len() == 2 => parts[1]
            .parse()
            .map_err(|_| LabelError("invalid snapshot channel type"))?,
        _ => return Err(LabelError("snapshot requires exactly one channel type")),
    };
    if namespace_count != usize::from(!values.is_empty()) {
        return Err(LabelError(
            "snapshot namespace must occur once for nonempty labels only",
        ));
    }
    if !values.is_empty() && !matches!(channel_type, ChannelType::Stream | ChannelType::Forum) {
        return Err(LabelError("channel type does not support labels"));
    }
    LabelSet::new(values)
}
