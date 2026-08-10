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
      normalized.startsWith('error: too many subscriptions')) {
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
