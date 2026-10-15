part of 'relay_session.dart';

const _maxPendingRelayEvents = 2048;
const _relayEventVerificationBatchSize = 64;
// The relay caps frames at 512 KiB and event content at 256 KiB. This is a
// conservative upper bound for JSON-encoded bytes: a Dart UTF-16 code unit can
// expand to six bytes when JSON escaping is required.
const _maxRelayEventEncodedByteUpperBound = 4 * 1024 * 1024;
const _maxPendingRelayEventEncodedByteUpperBound = 32 * 1024 * 1024;
const _maxRelayEventNestingDepth = 64;

/// Serializes relay-event verification through one bounded background worker.
class _RelayEventVerifier {
  _RelayEventVerifier({
    required Future<List<Map<String, dynamic>?>> Function(List<Object?>)
    verifyBatch,
  }) : _verifyBatch = verifyBatch;

  final Future<List<Map<String, dynamic>?>> Function(List<Object?>)
  _verifyBatch;
  final Queue<_PendingRelayEventVerification> _pending = Queue();
  List<_PendingRelayEventVerification> _active = const [];
  Timer? _drainTimer;
  var _pendingCount = 0;
  var _pendingEncodedByteUpperBound = 0;
  var _draining = false;
  var _closed = false;

  Future<NostrEvent?> verify(Object? eventJson) async {
    final verified = await verifyAll([eventJson]);
    return verified.single;
  }

  Future<List<NostrEvent?>> verifyAll(List<Object?> eventJsons) {
    if (_closed) {
      return Future.error(StateError('Relay event verifier is closed'));
    }
    final encodedByteUpperBounds = <int?>[];
    var addedCount = 0;
    var addedEncodedByteUpperBound = 0;
    for (final eventJson in eventJsons) {
      final encodedByteUpperBound = _relayEventEncodedByteUpperBound(eventJson);
      encodedByteUpperBounds.add(encodedByteUpperBound);
      if (encodedByteUpperBound != null) {
        addedCount++;
        addedEncodedByteUpperBound += encodedByteUpperBound;
      }
    }
    if (addedCount > _maxPendingRelayEvents - _pendingCount ||
        addedEncodedByteUpperBound >
            _maxPendingRelayEventEncodedByteUpperBound -
                _pendingEncodedByteUpperBound) {
      return Future.error(StateError('Relay event verification queue is full'));
    }

    final futures = <Future<NostrEvent?>>[];
    for (var i = 0; i < eventJsons.length; i++) {
      final encodedByteUpperBound = encodedByteUpperBounds[i];
      if (encodedByteUpperBound == null) {
        futures.add(Future.value());
        continue;
      }
      final pending = _PendingRelayEventVerification(
        eventJsons[i],
        encodedByteUpperBound,
      );
      _pending.add(pending);
      _pendingCount++;
      _pendingEncodedByteUpperBound += encodedByteUpperBound;
      futures.add(pending.completer.future);
    }
    _scheduleDrain();
    return Future.wait(futures);
  }

  void close() {
    if (_closed) return;
    _closed = true;
    _drainTimer?.cancel();
    _drainTimer = null;
    final error = StateError('Relay event verifier is closed');
    for (final pending in [..._active, ..._pending]) {
      _completeError(pending, error, StackTrace.current);
    }
    _pending.clear();
  }

  void _scheduleDrain() {
    if (_closed || _draining || _drainTimer != null || _pending.isEmpty) {
      return;
    }
    // A zero-duration timer coalesces adjacent WebSocket frames into batches.
    _drainTimer = Timer(Duration.zero, () {
      _drainTimer = null;
      unawaited(_drain());
    });
  }

  Future<void> _drain() async {
    if (_closed || _draining) return;
    _draining = true;
    try {
      while (!_closed && _pending.isNotEmpty) {
        final batch = <_PendingRelayEventVerification>[];
        while (batch.length < _relayEventVerificationBatchSize &&
            _pending.isNotEmpty) {
          batch.add(_pending.removeFirst());
        }
        _active = batch;

        try {
          final verified = await _verifyBatch([
            for (final pending in batch) pending.eventJson,
          ]);
          if (verified.length != batch.length) {
            throw StateError(
              'Relay event verifier returned ${verified.length} results '
              'for ${batch.length} events',
            );
          }
          for (var i = 0; i < batch.length; i++) {
            final eventJson = verified[i];
            _complete(
              batch[i],
              eventJson == null ? null : NostrEvent.fromJson(eventJson),
            );
          }
        } catch (error, stackTrace) {
          for (final pending in batch) {
            _completeError(pending, error, stackTrace);
          }
        } finally {
          _active = const [];
        }
      }
    } finally {
      _draining = false;
      _scheduleDrain();
    }
  }

  void _complete(_PendingRelayEventVerification pending, NostrEvent? event) {
    if (pending.completer.isCompleted) return;
    _pendingCount--;
    _pendingEncodedByteUpperBound -= pending.encodedByteUpperBound;
    pending.completer.complete(event);
  }

  void _completeError(
    _PendingRelayEventVerification pending,
    Object error,
    StackTrace stackTrace,
  ) {
    if (pending.completer.isCompleted) return;
    _pendingCount--;
    _pendingEncodedByteUpperBound -= pending.encodedByteUpperBound;
    pending.completer.completeError(error, stackTrace);
  }
}

class _PendingRelayEventVerification {
  _PendingRelayEventVerification(this.eventJson, this.encodedByteUpperBound);

  final Object? eventJson;
  final int encodedByteUpperBound;
  final Completer<NostrEvent?> completer = Completer();
}

int? _relayEventEncodedByteUpperBound(Object? value) {
  return _encodedByteUpperBound(value, _maxRelayEventEncodedByteUpperBound, 0);
}

int? _encodedByteUpperBound(Object? value, int remaining, int depth) {
  if (remaining < 0 || depth > _maxRelayEventNestingDepth) return null;
  if (value == null || value is bool || value is num) {
    return remaining >= 32 ? 32 : null;
  }
  if (value is String) {
    final bytes = 2 + value.length * 6;
    return bytes <= remaining ? bytes : null;
  }
  if (value is List) {
    var total = 2;
    for (final element in value) {
      final elementBytes = _encodedByteUpperBound(
        element,
        remaining - total - 1,
        depth + 1,
      );
      if (elementBytes == null) return null;
      total += elementBytes + 1;
    }
    return total <= remaining ? total : null;
  }
  if (value is Map) {
    var total = 2;
    for (final entry in value.entries) {
      if (entry.key is! String) return null;
      final keyBytes = _encodedByteUpperBound(
        entry.key,
        remaining - total - 1,
        depth + 1,
      );
      if (keyBytes == null) return null;
      total += keyBytes + 1;
      final valueBytes = _encodedByteUpperBound(
        entry.value,
        remaining - total - 1,
        depth + 1,
      );
      if (valueBytes == null) return null;
      total += valueBytes + 1;
    }
    return total <= remaining ? total : null;
  }
  return null;
}

Future<List<Map<String, dynamic>?>> _verifyRelayEventBatchInBackground(
  List<Object?> eventJsons,
) => compute(_verifyRelayEventBatch, eventJsons);

List<Map<String, dynamic>?> _verifyRelayEventBatch(List<Object?> eventJsons) {
  return [for (final eventJson in eventJsons) _verifyRelayEvent(eventJson)];
}

Map<String, dynamic>? _verifyRelayEvent(Object? eventJson) {
  try {
    // Event.fromJson verifies the canonical seven-field ID and BIP-340
    // signature. Its Schnorr boundary strictly decodes the complete key,
    // message, and signature widths before invoking bip340 verification.
    return nostr.Event.fromJson(jsonEncode(eventJson)).toMap();
  } catch (_) {
    return null;
  }
}
