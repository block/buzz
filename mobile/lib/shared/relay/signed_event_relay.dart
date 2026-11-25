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
    return _session.publish(nostrEvent, mayPublish: mayPublish);
  }
}

/// Publishes one signed event over a short-lived authenticated NIP-42 socket.
///
/// This is used for community-removal tombstones because the community being
/// removed is not necessarily the app's active relay session.
Future<NostrEvent> submitSignedEventOnce({
  required String wsUrl,
  required String nsec,
  required int kind,
  required String content,
  required List<List<String>> tags,
  int? createdAt,
  Duration timeout = const Duration(seconds: 12),
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
      ] when eventId == event.id) {
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
          result.completeError(_SignedEventRejection(detail), callSite);
        }
      }
    },
    onConnected: () {
      if (!result.isCompleted) socket.send(['EVENT', event.toJson()]);
    },
    onDisconnected: (error) {
      if (result.isCompleted) return;
      // The socket's own authentication deadline is shorter than [timeout],
      // so its plain timeout is typed here as this call's.
      if (error is TimeoutException && error is! RelayCallError) {
        result.completeError(
          RelayTimeoutException(
            RelayCall.publish,
            error.message ?? 'Relay did not answer',
            error.duration,
          ),
          callSite,
        );
        return;
      }
      result.completeError(error ?? Exception('Relay disconnected'), callSite);
    },
  );
  socket.headers = headers;
  socket.diagnostics = reporter;
  // The deadline covers connecting and authenticating too. Once it passes the
  // event is never sent, and its timeout goes to the caller, never to the
  // zone as an uncaught error.
  final deadline = Timer(timeout, () {
    if (result.isCompleted) return;
    result.completeError(
      RelayTimeoutException(
        RelayCall.publish,
        'Relay did not answer within $timeout',
        timeout,
      ),
      callSite,
    );
  });
  try {
    // Whichever settles first: a failed connect, or the result (which the
    // deadline settles if nothing else does). Future.any listens to both, so
    // the other one's later error is not uncaught.
    return await Future.any([
      socket.connect().then((_) => result.future),
      result.future,
    ]);
  } finally {
    deadline.cancel();
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
