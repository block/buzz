import 'dart:async';
import 'dart:math';

/// Creates a timer used by [RelayRateLimitGate].
typedef RelayTimerFactory =
    Timer Function(Duration duration, void Function() callback);

/// Session-owned gate that pauses relay requests after back-pressure.
class RelayRateLimitGate {
  /// Default gate duration when the relay omits a positive retry hint.
  static const defaultRetrySeconds = 10;

  /// Longest retry hint accepted from a relay response.
  static const maxRetrySeconds = 300;

  RelayRateLimitGate({
    DateTime Function()? now,
    RelayTimerFactory timerFactory = Timer.new,
  }) : _now = now ?? DateTime.now,
       _timerFactory = timerFactory;

  final DateTime Function() _now;
  final RelayTimerFactory _timerFactory;
  DateTime? _expiresAt;
  Timer? _timer;
  Completer<void>? _completer;

  /// Whether a rate-limit window is currently active.
  bool get isActive {
    final expiresAt = _expiresAt;
    return expiresAt != null && _now().isBefore(expiresAt);
  }

  /// Activates or extends the gate without shrinking an existing window.
  void activate(int? retryInSeconds) {
    final seconds = retryInSeconds != null && retryInSeconds > 0
        ? min(retryInSeconds, maxRetrySeconds)
        : defaultRetrySeconds;
    final duration = Duration(seconds: seconds);
    final newExpiry = _now().add(duration);
    final currentExpiry = _expiresAt;
    if (currentExpiry != null && !newExpiry.isAfter(currentExpiry)) return;

    _expiresAt = newExpiry;
    _timer?.cancel();
    _completer ??= Completer<void>();
    _timer = _timerFactory(duration, _expire);
  }

  /// Resolves when the active rate-limit window expires.
  Future<void> wait() {
    if (!isActive) return Future.value();
    return _completer!.future;
  }

  /// Milliseconds remaining in the active window, or zero when inactive.
  int remainingMs() {
    final expiresAt = _expiresAt;
    if (expiresAt == null) return 0;
    return max(0, expiresAt.difference(_now()).inMilliseconds);
  }

  /// Clears the gate and releases all current waiters.
  void reset() {
    _timer?.cancel();
    _timer = null;
    _expiresAt = null;
    final completer = _completer;
    _completer = null;
    if (completer != null && !completer.isCompleted) completer.complete();
  }

  void _expire() {
    _timer = null;
    _expiresAt = null;
    final completer = _completer;
    _completer = null;
    if (completer != null && !completer.isCompleted) completer.complete();
  }
}
