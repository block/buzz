import 'package:flutter/foundation.dart';

/// Serializes camera teardown so replacement sessions never overlap.
///
/// A native disposal failure is intentionally contained: callers can proceed
/// with the next camera session after the failed release has settled.
@visibleForTesting
class CameraDisposalBarrier {
  Future<void> _pending = Future<void>.value();

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
