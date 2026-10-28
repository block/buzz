import 'dart:math';

/// Reconnect delay before the first retry, in milliseconds.
const relayReconnectBaseDelayMs = 1000;

/// Ceiling for a reconnect delay, in milliseconds.
const relayReconnectMaxDelayMs = 30000;

/// How long a connection must stay up before a drop stops counting as part of
/// a bounce loop and the backoff returns to [relayReconnectBaseDelayMs]. Matches
/// desktop's `BACKOFF_RESET_STABLE_MS`.
const relayStableConnectionDuration = Duration(seconds: 60);

/// Width of the uniform window the first reconnect after a server restart
/// (close code 1012) or shutdown (1001) is spread over. The relay already
/// spreads those closes over 10s; this spreads the arrivals too.
const relayServiceRestartSpreadMs = 10000;

/// Seconds a replay reaches behind the newest delivered event.
const relayReplaySkewSeconds = 5;

/// Seconds a replay reaches behind the moment a subscription's connection
/// closed. Absorbs client/relay clock skew and `created_at` jitter.
const relayLiveWindowSkewSeconds = 60;

/// Whether [closeCode] says the relay is restarting or going away.
bool isRelayServiceRestartClose(int? closeCode) =>
    closeCode == 1012 || closeCode == 1001;

/// The delay before the next reconnect attempt.
///
/// [backoffMs] is the current exponential backoff. The delay is jittered
/// symmetrically by +/-25% and capped at [relayReconnectMaxDelayMs]. After a
/// service-restart close the first attempt (backoff still at the base) is
/// instead uniform over [0, [relayServiceRestartSpreadMs]]; a client already
/// backing off waits the longer of the two so a bounce loop cannot spin.
///
/// [jitterUnit] and [spreadUnit] are uniform draws in [0, 1).
Duration relayReconnectDelay({
  required int backoffMs,
  required int? closeCode,
  required double jitterUnit,
  required double spreadUnit,
}) {
  final jittered = min(
    (backoffMs * (0.75 + jitterUnit * 0.5)).round(),
    relayReconnectMaxDelayMs,
  );
  if (!isRelayServiceRestartClose(closeCode)) {
    return Duration(milliseconds: jittered);
  }
  final spread = (spreadUnit * relayServiceRestartSpreadMs).round();
  return Duration(
    milliseconds: backoffMs > relayReconnectBaseDelayMs
        ? max(jittered, spread)
        : spread,
  );
}

/// Whether a connection that stayed up for [uptime] was stable.
bool isRelayConnectionStable(Duration uptime) =>
    uptime >= relayStableConnectionDuration;

/// The `since` a live subscription replays with after a reconnect, or null to
/// replay the original filter unchanged.
///
/// [lastSeenCreatedAt] is the newest event delivered. [liveUntil] is the
/// client clock, in seconds, when the subscription's last connection closed,
/// set only if that subscription reached EOSE on it: everything stored was
/// delivered by EOSE and everything after it arrived live until the close.
/// The result is the latest of the original `since`,
/// `lastSeenCreatedAt - 5s` and `liveUntil - 60s`, never past [until].
int? relayResumeSince({
  required int? originalSince,
  required int? until,
  required int? lastSeenCreatedAt,
  required int? liveUntil,
}) {
  if (lastSeenCreatedAt == null && liveUntil == null) return originalSince;
  var since = originalSince ?? 0;
  if (lastSeenCreatedAt != null) {
    since = max(since, lastSeenCreatedAt - relayReplaySkewSeconds);
  }
  if (liveUntil != null) {
    since = max(since, liveUntil - relayLiveWindowSkewSeconds);
  }
  if (until != null) since = min(since, until);
  return max(0, since);
}
