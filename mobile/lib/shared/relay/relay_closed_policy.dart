import 'dart:convert';

import 'package:hooks_riverpod/hooks_riverpod.dart';

import 'relay_client.dart';

// Shared constant so [isRelayDeadlineError] and [classifyRelayClosed] cannot
// drift apart.
const _queryTimedOutPrefix = 'error: query timed out';

/// Returns true when [error] is a relay server-side statement deadline that
/// cannot be resolved by retrying the same query.
///
/// Two surfaces carry the same deadline signal:
/// - HTTP `/query` or `/count`: [RelayException] with status 503 and a JSON
///   body containing `"error": "query timed out"`.
/// - WebSocket CLOSED: a plain [Exception] whose message is
///   `"error: query timed out"` (the relay's stable CLOSED reason string).
///
/// Both surfaces are checked so the caller (e.g. a Riverpod retry hook) does
/// not need to distinguish which transport produced the error.
bool isRelayDeadlineError(Object error) {
  if (error is RelayException && error.statusCode == 503) {
    try {
      final decoded = jsonDecode(error.body);
      if (decoded is Map<String, dynamic>) {
        final message = decoded['error'];
        if (message is String &&
            message.trim().toLowerCase() == 'query timed out') {
          return true;
        }
      }
    } on FormatException {
      // Non-JSON 503 — not a deadline.
    }
    return false;
  }
  // WebSocket history CLOSED: Exception("error: query timed out")
  // Strip the "Exception: " prefix Flutter adds to Exception.toString(), then
  // check via classifyRelayClosed so this path shares the same source of truth.
  var message = error.toString().trim().toLowerCase();
  const prefix = 'exception: ';
  if (message.startsWith(prefix)) message = message.substring(prefix.length);
  return classifyRelayClosed(message) == RelayClosedClass.terminal &&
      message.startsWith(_queryTimedOutPrefix);
}

/// Recovery policy for a relay `CLOSED` subscription message.
enum RelayClosedClass {
  /// A transient failure that may recover when the same REQ is retried.
  retryable,

  /// Relay back-pressure that must also arm the shared request gate.
  rateLimited,

  /// An authorization, access, or filter failure that cannot recover unchanged.
  terminal,
}

/// Classifies whether a relay `CLOSED` message should be retried.
RelayClosedClass classifyRelayClosed(String message) {
  final normalized = message.trim().toLowerCase();
  if (normalized.startsWith('rate-limited:')) {
    return RelayClosedClass.rateLimited;
  }
  if (normalized.startsWith('restricted:') ||
      normalized.startsWith('auth-required:') ||
      normalized.startsWith('blocked:') ||
      normalized.startsWith('invalid:') ||
      normalized.startsWith('pow:') ||
      normalized.startsWith('duplicate:') ||
      normalized.startsWith('unsupported:') ||
      normalized.startsWith('error: mixed search') ||
      normalized.startsWith('error: too many subscriptions') ||
      // Server statement deadline: re-sending the same REQ re-runs the same
      // slow query.
      normalized.startsWith(_queryTimedOutPrefix)) {
    return RelayClosedClass.terminal;
  }
  return RelayClosedClass.retryable;
}

final _rateLimitRetryPattern = RegExp(r'retry in (\d+)s', caseSensitive: false);

/// Parses the relay's canonical `retry in Ns` hint, when present.
int? parseRateLimitRetrySeconds(String message) {
  final match = _rateLimitRetryPattern.firstMatch(message);
  return match == null ? null : int.tryParse(match.group(1)!);
}

/// Whether [value] has settled on a relay deadline.
///
/// Automatic refresh owners (timers, reconnect, live-event invalidation) must
/// not re-send a request in this state; only an explicit user action may.
bool isSettledRelayDeadline(AsyncValue<Object?> value) =>
    value.hasError && !value.isLoading && isRelayDeadlineError(value.error!);

/// Production Riverpod retry policy for relay providers.
///
/// A relay statement deadline cannot resolve on retry — the same expensive
/// query would re-run and hit the same limit. Returns `null` (no retry) for
/// deadline errors and falls through to [ProviderContainer.defaultRetry] for
/// everything else.
///
/// Pass this to [ProviderScope.retry] at the root of the widget tree.
Duration? relayProviderRetry(int retryCount, Object error) {
  if (isRelayDeadlineError(error)) return null;
  return ProviderContainer.defaultRetry(retryCount, error);
}
