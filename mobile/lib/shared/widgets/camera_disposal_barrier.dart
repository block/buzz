import 'dart:async';

/// Serializes camera teardown so replacement sessions never overlap.
///
/// A native disposal failure is intentionally contained: callers can proceed
/// with the next camera session after the failed release has settled.
class CameraDisposalBarrier {
  Future<void> _pending = Future<void>.value();

  /// Reserves camera ownership while a controller is initializing.
  CameraDisposalReservation reserve() {
    final reservation = CameraDisposalReservation._(_pending);
    _pending = reservation._settled;
    return reservation;
  }

  /// Completes after earlier and [dispose] callbacks have settled.
  ///
  /// Exceptions from [dispose] are handled so they cannot prevent a later
  /// replacement session from acquiring the camera.
  Future<void> release(Future<void> Function() dispose) {
    final previous = _pending;
    final release = () async {
      await previous;
      try {
        await dispose();
      } catch (_) {
        // A failed native release must not permanently block camera recovery.
      }
    }();
    _pending = release;
    return release;
  }

  /// Completes once all scheduled teardown work has settled.
  Future<void> get settled => _pending;
}

/// Owns one in-flight camera initialization in a [CameraDisposalBarrier].
class CameraDisposalReservation {
  CameraDisposalReservation._(this._previous) {
    _settled = () async {
      await _previous;
      await _completion.future;
    }();
  }

  final Future<void> _previous;
  final _completion = Completer<void>();
  late final Future<void> _settled;

  /// Waits for older camera teardown before this candidate initializes.
  Future<void> get ready => _previous;

  /// Releases this reservation after a successful initialization.
  void complete() {
    if (!_completion.isCompleted) _completion.complete();
  }

  /// Disposes a failed or cancelled candidate before allowing replacement.
  Future<void> dispose(Future<void> Function() release) async {
    await _previous;
    try {
      await release();
    } catch (_) {
      // A failed native release must not permanently block camera recovery.
    } finally {
      complete();
    }
  }
}
