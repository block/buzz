part of 'relay_session.dart';

/// Inputs passed to a query-auth signing worker.
typedef RelayQueryAuthRequest = ({
  String method,
  String url,
  List<int> bodyBytes,
  String? nsec,
});

/// Serializes query authentication off the UI isolate, with bounded admission.
///
/// Proofs are minted when their job starts, not while waiting in the queue.
/// Retiring the session cancels queued jobs and discards active results.
class RelayQuerySigner {
  /// Creates a signer; tests may supply a controlled asynchronous worker.
  RelayQuerySigner({
    @visibleForTesting Future<String> Function(RelayQueryAuthRequest)? sign,
  }) : _sign = sign ?? _signQueryInBackground;

  final Future<String> Function(RelayQueryAuthRequest) _sign;
  static const _maxPending = 64;
  final _queue = Queue<_RelayQuerySigningJob>();
  bool _running = false;
  int _generation = 0;

  /// Signs one request, rejecting excess work without retaining another job.
  Future<String> sign(RelayQueryAuthRequest request) {
    if (_queue.length + (_running ? 1 : 0) >= _maxPending) {
      return Future.error(StateError('Relay query signing queue is full'));
    }
    final job = _RelayQuerySigningJob(request, _generation);
    _queue.addLast(job);
    if (!_running) unawaited(_drain());
    return job.result.future;
  }

  Future<void> _drain() async {
    _running = true;
    try {
      while (_queue.isNotEmpty) {
        final job = _queue.removeFirst();
        try {
          final header = await _sign(job.request);
          if (job.generation != _generation) {
            throw StateError('Relay query belongs to a retired session');
          }
          job.result.complete(header);
        } catch (error, stack) {
          job.result.completeError(error, stack);
        }
      }
    } finally {
      _running = false;
    }
  }

  /// Cancels queued work immediately, preserving the active worker barrier.
  void retire() {
    _generation++;
    while (_queue.isNotEmpty) {
      _queue.removeFirst().result.completeError(
        StateError('Relay query belongs to a retired session'),
      );
    }
  }
}

class _RelayQuerySigningJob {
  _RelayQuerySigningJob(this.request, this.generation);

  final RelayQueryAuthRequest request;
  final int generation;
  final result = Completer<String>();
}

Future<String> _signQueryInBackground(RelayQueryAuthRequest request) =>
    compute(_signQuery, request, debugLabel: 'relay-query-auth');

String _signQuery(RelayQueryAuthRequest request) => buildNip98AuthHeader(
  method: request.method,
  url: request.url,
  bodyBytes: request.bodyBytes,
  nsec: request.nsec,
);

String buildNip98AuthHeader({
  required String method,
  required String url,
  required List<int> bodyBytes,
  required String? nsec,
}) {
  if (nsec == null || nsec.isEmpty) {
    throw Exception('Cannot query relay: no signing key available');
  }
  final privkeyHex = nostr.Nip19.decode(payload: nsec).data;
  if (privkeyHex.isEmpty) {
    throw Exception('Invalid nsec');
  }
  final payloadHash = SHA256Digest()
      .process(Uint8List.fromList(bodyBytes))
      .map((byte) => byte.toRadixString(16).padLeft(2, '0'))
      .join();
  final event = nostr.Event.from(
    kind: 27235,
    content: '',
    tags: [
      ['u', url],
      ['method', method.toUpperCase()],
      ['payload', payloadHash],
      ['nonce', const Uuid().v4()],
    ],
    secretKey: privkeyHex,
    verify: false,
  );
  return 'Nostr ${base64.encode(utf8.encode(event.toJson()))}';
}
