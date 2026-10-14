import 'read_state_time.dart';

/// How far ahead of local time a read marker may plausibly sit.
///
/// This matches the relay's own ingest gate: `MAX_TIMESTAMP_DRIFT_SECS = 900`
/// in `crates/buzz-relay/src/handlers/ingest.rs`, applied symmetrically to every
/// event after signature verification and before any kind-specific branch, on
/// both the WebSocket and HTTP paths. 900 is therefore the widest skew an
/// ordinary message can carry and still be stored.
///
/// It is deliberately NOT `MAX_COMMAND_SKEW_SECS` (120) from
/// `handlers/moderation_commands.rs`: that is a replay window for moderation
/// kinds that are never stored. Using the smaller number would reject timestamps
/// the relay legitimately accepted, leaving a channel showing a phantom unread
/// badge that re-arms on every open.
///
/// Distinct from [readStateMaxClockDriftSeconds], which governs publish
/// sequencing rather than marker validity.
const readMarkerMaxSkewSeconds = 900;

/// Repairs a read marker that sits implausibly far in the future.
///
/// Unreadness is decided by `createdAt > readAt`, so a marker pushed past the
/// present suppresses every genuinely newer message until wall-clock catches up.
/// Markers are monotonic — see `_advanceContext` here and `copyWithContext` in
/// `read_state_provider.dart` — so a poisoned value cannot be lowered again by
/// ordinary use, and it survives persistence and sync.
///
/// Implausible values are clamped to `now`, **never dropped**. Dropping would be
/// unsafe: plausibility is judged against the *local* clock, which cannot tell
/// "their clock is fast" from "mine is slow". A device booting before its first
/// NTP sync, resuming from suspend, or holding a dead RTC would judge every
/// stored marker implausible and discard read state it has no way to recover.
/// Clamping keeps the invariant that matters — a marker at `now` can never hide
/// a message arriving after `now` — with no data loss, and it corrects itself on
/// the next run if the local clock was the thing at fault.
///
/// Clamping only ever *lowers* a marker, so it strictly widens what counts as
/// unread. It can never cause a channel marker to swallow a thread reply that
/// should have stayed unread.
int clampReadMarker(int unixSeconds, {int? nowSeconds}) {
  final now = nowSeconds ?? currentUnixSeconds();
  if (unixSeconds > now + readMarkerMaxSkewSeconds) {
    return now;
  }
  return unixSeconds;
}

/// [clampReadMarker] across a whole context map, for stored and synced state.
///
/// Keys are preserved exactly; only values are repaired. Nothing is pruned.
Map<String, int> clampReadMarkers(
  Map<String, int> contexts, {
  int? nowSeconds,
}) {
  final now = nowSeconds ?? currentUnixSeconds();
  return <String, int>{
    for (final entry in contexts.entries)
      entry.key: clampReadMarker(entry.value, nowSeconds: now),
  };
}
