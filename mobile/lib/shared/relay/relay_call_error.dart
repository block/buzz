import 'dart:async';

/// The relay call an error came from.
enum RelayCall {
  /// An EVENT sent over the socket and acknowledged with OK.
  publish,

  /// A one-shot query over the relay's HTTP bridge.
  query,

  /// A one-shot REQ over the socket, read until EOSE.
  history,

  /// A live REQ, up to its first EOSE.
  subscribe,
}

/// The reason class of a failed relay call: the relay's own reason prefix, or
/// a timeout. Never the reason text, which is free-form and can quote what
/// the app sent.
enum RelayReason {
  invalid('invalid'),
  rateLimited('rate-limited'),
  restricted('restricted'),
  authRequired('auth-required'),
  blocked('blocked'),
  duplicate('duplicate'),
  pow('pow'),
  unsupported('unsupported'),
  error('error'),

  /// The call timed out here, before the relay answered.
  timeout('timeout'),

  /// A reason with no known prefix.
  other('other');

  const RelayReason(this.tag);

  /// The wording that leaves the device.
  final String tag;

  static final _prefixes = {
    for (final reason in values)
      if (reason != timeout && reason != other) reason.tag: reason,
  };

  /// The class of a relay `OK` or `CLOSED` [message]: its prefix, up to the
  /// first colon, when that is one of the relay's.
  static RelayReason fromMessage(String message) {
    final colon = message.indexOf(':');
    if (colon < 0) return other;
    return _prefixes[message.substring(0, colon).trim().toLowerCase()] ?? other;
  }

  /// The class of an HTTP error [status] from the relay's `/query`. Its body
  /// is not read: the status alone says rate limit, refusal or server error.
  static RelayReason fromHttpStatus(int status) => switch (status) {
    429 => rateLimited,
    401 => authRequired,
    403 => restricted,
    >= 500 && < 600 => error,
    _ => other,
  };
}

/// An error from a relay call that says which call and why.
///
/// Diagnostic reports of these carry [call] and [reason] as tags, because the
/// error's own text is removed before a report leaves the device.
abstract interface class RelayCallError implements Exception {
  RelayCall get call;
  RelayReason get reason;
}

/// A relay call timed out before the relay answered.
class RelayTimeoutException extends TimeoutException implements RelayCallError {
  RelayTimeoutException(this.call, String message, [Duration? duration])
    : super(message, duration);

  @override
  final RelayCall call;

  @override
  RelayReason get reason => RelayReason.timeout;

  /// The text without the duration, which [message] already states. Screens
  /// show this text.
  @override
  String toString() => 'TimeoutException: $message';
}

/// The relay closed a history or live subscription with [message].
class RelayClosedException implements RelayCallError {
  RelayClosedException(this.call, this.message);

  @override
  final RelayCall call;

  /// The relay's `CLOSED` reason. Callers read its text.
  final String message;

  @override
  RelayReason get reason => RelayReason.fromMessage(message);

  @override
  String toString() => 'Exception: $message';
}
