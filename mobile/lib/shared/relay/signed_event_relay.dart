import 'dart:async';

import 'package:nostr/nostr.dart' as nostr;

import '../diagnostics/diagnostic_reporter.dart';
import 'nostr_models.dart';
import 'relay_call_error.dart';
import 'relay_closed_policy.dart';
import 'relay_session.dart';
import 'relay_socket.dart';
import 'package:buzz/shared/crypto/event_signing.dart';

/// Signs and submits Nostr events through the relay WebSocket connection.
class SignedEventRelay {
  final RelaySessionNotifier _session;
  final String? _nsec;

  SignedEventRelay({
    required RelaySessionNotifier session,
    required String? nsec,
  }) : _session = session,
       _nsec = nsec;

  /// The hex pubkey derived from the signing key, or null if no key.
  String? get pubkey {
    final nsec = _nsec;
    if (nsec == null || nsec.isEmpty) return null;
    final privkeyHex = nostr.Nip19.decode(payload: nsec).data;
    if (privkeyHex.isEmpty) return null;
    return derivePublicKeyHex(privkeyHex);
  }

  /// Sign and submit an event. Returns the relay's OK response as a [NostrEvent]
  /// whose `content` field contains the OK message (e.g. `"response:{...}"`
  /// for command kinds). [mayPublish] is asked right before the event goes
  /// out, after every wait.
  Future<NostrEvent> submit({
    required int kind,
    required String content,
    required List<List<String>> tags,
    int? createdAt,
    FutureOr<void> Function(NostrEvent event)? onSigned,
    Future<bool> Function()? mayPublish,
  }) async {
    // Taken before the first await, while the caller is still on the stack: a
    // caller that doesn't await this call is gone from it afterwards.
    final callSite = StackTrace.current;
    final nsec = _nsec;
    if (nsec == null || nsec.isEmpty) {
      throw Exception('Cannot submit event: no signing key available');
    }

    final privkeyHex = nostr.Nip19.decode(payload: nsec).data;
    if (privkeyHex.isEmpty) {
      throw Exception('Invalid nsec');
    }

    final event = signNostrEvent(
      kind: kind,
      content: content,
      tags: tags,
      secretKey: privkeyHex,
      createdAt: createdAt,
    );

    final nostrEvent = NostrEvent.fromJson(event.toMap());
    // Durable callers must finish journaling the exact signed bytes before
    // the relay can accept an event whose acknowledgement might be lost.
    await onSigned?.call(nostrEvent);
    try {
      return await _session.publish(nostrEvent, mayPublish: mayPublish);
    } on RelayCallError catch (error) {
      // The session's own call site starts after the wait for `onSigned`.
      Error.throwWithStackTrace(error, trimCallSite(callSite));
    }
  }
}

/// The EVENT went out on the socket but the relay's OK never came back, so the
/// relay may or may not have stored it.
class SignedEventUnacknowledged implements Exception {
  const SignedEventUnacknowledged(this.cause);

  /// Why no OK arrived: a timeout or a dropped connection.
  final Object cause;

  @override
  String toString() => 'Relay did not acknowledge the event: $cause';
}

/// Publishes one signed event over a short-lived authenticated NIP-42 socket.
///
/// This is used for community-removal tombstones because the community being
/// removed is not necessarily the app's active relay session.
///
/// The attempt ends at [timeout] after the call, or at [deadline] when that
/// comes first. At that point the socket closes, and an EVENT not yet sent
/// never is. A failure after the EVENT went out, other than the relay's
/// refusal, is a [SignedEventUnacknowledged].
Future<NostrEvent> submitSignedEventOnce({
  required String wsUrl,
  required String nsec,
  required int kind,
  required String content,
  required List<List<String>> tags,
  int? createdAt,
  Duration timeout = const Duration(seconds: 12),
  DateTime? deadline,
  Map<String, String> headers = const {},
  DiagnosticReporter diagnostics = const NoopDiagnosticReporter(),
}) async {
  final privateKey = nostr.Nip19.decode(payload: nsec).data;
  if (privateKey.isEmpty) throw const FormatException('Invalid nsec');
  final signed = signNostrEvent(
    kind: kind,
    content: content,
    tags: tags,
    secretKey: privateKey,
    createdAt: createdAt,
  );
  final event = NostrEvent.fromJson(signed.toMap());
  // The signer may be a community that isn't active, such as a removed one's
  // push tombstone; its reports must not name the active member.
  final reporter = SignerDiagnosticReporter(diagnostics, signer: event.pubkey);
  final result = Completer<NostrEvent>();
  // Errors completed later, from a socket callback or timer, carry the stack
  // of the caller, so an escaped error's report reaches app code.
  final callSite = StackTrace.current;
  var sent = false;
  var expired = false;
  void fail(Object error) {
    if (result.isCompleted) return;
    result.completeErrorAt(
      sent ? SignedEventUnacknowledged(error) : error,
      callSite,
    );
  }

  late final RelaySocket socket;
  socket = RelaySocket(
    wsUrl: wsUrl,
    nsec: nsec,
    onMessage: (message) {
      if (message case [
        'OK',
        final String eventId,
        final bool accepted,
        final String detail,
        ...,
      ] when eventId == event.id && !result.isCompleted) {
        if (accepted) {
          result.complete(
            NostrEvent(
              id: event.id,
              pubkey: event.pubkey,
              createdAt: event.createdAt,
              kind: event.kind,
              tags: event.tags,
              content: detail,
              sig: event.sig,
            ),
          );
        } else {
          if (isInvalidRequestRejection(detail)) {
            reporter.reportError(
              RelayInvalidRequest(),
              StackTrace.current,
              operation: DiagnosticOperation.relayRejection,
            );
          }
          result.completeErrorAt(_SignedEventRejection(detail), callSite);
        }
      }
    },
    onConnected: () {
      if (expired || result.isCompleted) return;
      sent = true;
      socket.send(['EVENT', event.toJson()]);
    },
    onDisconnected: (error) {
      // The socket's own authentication deadline is shorter than [timeout],
      // so its plain timeout is typed here as this call's.
      if (error is TimeoutException && error is! RelayCallError) {
        fail(
          RelayTimeoutException(
            RelayCall.publish,
            error.message ?? 'Relay did not answer',
            error.duration,
          ),
        );
        return;
      }
      fail(error ?? Exception('Relay disconnected'));
    },
  );
  socket.headers = headers;
  socket.diagnostics = reporter;
  var limit = timeout;
  if (deadline != null) {
    final remaining = deadline.difference(DateTime.now());
    if (remaining < limit) limit = remaining;
  }
  if (limit.isNegative) limit = Duration.zero;
  final timer = Timer(limit, () {
    expired = true;
    fail(
      RelayTimeoutException(
        RelayCall.publish,
        'Relay did not answer within $limit',
        limit,
      ),
    );
    unawaited(socket.disconnect());
  });
  try {
    unawaited(socket.connect().catchError(fail));
    return await result.future;
  } finally {
    timer.cancel();
    await socket.disconnect();
  }
}

/// The relay answered a one-shot EVENT with OK false and [detail]. Its text
/// names the refusal for callers; its reason class is the prefix of [detail].
class _SignedEventRejection extends RelayEventRejectedException {
  _SignedEventRejection(String detail)
    : _reason = RelayReason.fromMessage(detail),
      super('Relay rejected event: $detail');

  final RelayReason _reason;

  @override
  RelayReason get reason => _reason;
}
