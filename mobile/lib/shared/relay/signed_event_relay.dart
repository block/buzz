import 'dart:async';

import 'package:nostr/nostr.dart' as nostr;

import '../auth/event_signer.dart';

import 'nostr_models.dart';
import 'relay_session.dart';
import 'relay_socket.dart';

/// Signs and submits Nostr events through the relay WebSocket connection.
///
/// Bound to the session scope at construction. Recreate after a dependency
/// rebuild. Signing completions in retired scopes throw
/// [RelaySessionSupersededError] without invoking [submit]'s callback or
/// publishing into the replacement scope.
class SignedEventRelay {
  final RelaySessionNotifier _session;
  final RelaySessionLease _lease;
  final String? _nsec;
  final EventSigner? _signer;

  SignedEventRelay({
    required RelaySessionNotifier session,
    required String? nsec,
  }) : _session = session,
       _lease = session.captureLease(),
       _nsec = nsec,
       _signer = null;

  /// Submit with an explicit signer snapshot, without access to a local secret.
  SignedEventRelay.withSigner({
    required RelaySessionNotifier session,
    required EventSigner signer,
  }) : _session = session,
       _lease = session.captureLease(),
       _signer = signer,
       _nsec = null;

  /// The hex pubkey derived from the signing key, or null if no key.
  String? get pubkey {
    if (_signer case final signer?) return signer.publicKey;
    final nsec = _nsec;
    if (nsec == null || nsec.isEmpty) return null;
    final privkeyHex = nostr.Nip19.decode(payload: nsec).data;
    if (privkeyHex.isEmpty) return null;
    return nostr.Keys(privkeyHex).public;
  }

  /// Sign and submit an event. Returns the relay's OK response as a [NostrEvent]
  /// whose `content` field contains the OK message (e.g. `"response:{...}"`
  /// for command kinds).
  Future<NostrEvent> submit({
    required int kind,
    required String content,
    required List<List<String>> tags,
    int? createdAt,
    void Function(NostrEvent event)? onSigned,
  }) async {
    _lease.ensureCurrent();
    final EventSigner signer;
    if (_signer case final supplied?) {
      signer = supplied;
    } else {
      final nsec = _nsec;
      if (nsec == null || nsec.isEmpty) {
        throw Exception('Cannot submit event: no signing key available');
      }
      final privkeyHex = nostr.Nip19.decode(payload: nsec).data;
      if (privkeyHex.isEmpty) throw Exception('Invalid nsec');
      signer = LocalEventSigner(privkeyHex);
    }

    final event = await signEvent(
      kind: kind,
      content: content,
      tags: tags,
      signer: signer,
      createdAt: createdAt,
    );

    final nostrEvent = NostrEvent.fromJson(event.toMap());
    _lease.ensureCurrent();
    onSigned?.call(nostrEvent);
    return _session.publish(nostrEvent, lease: _lease);
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
}) async {
  final privateKey = nostr.Nip19.decode(payload: nsec).data;
  if (privateKey.isEmpty) throw const FormatException('Invalid nsec');
  final signed = await signEvent(
    kind: kind,
    content: content,
    tags: tags,
    signer: LocalEventSigner(privateKey),
    createdAt: createdAt,
  );
  final event = NostrEvent.fromJson(signed.toMap());
  final result = Completer<NostrEvent>();
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
          result.completeError(Exception('Relay rejected event: $detail'));
        }
      }
    },
    onConnected: () => socket.send(['EVENT', event.toJson()]),
    onDisconnected: (error) {
      if (!result.isCompleted) {
        result.completeError(error ?? Exception('Relay disconnected'));
      }
    },
  );
  final resultFuture = result.future.timeout(timeout);
  try {
    await socket.connect();
    return await resultFuture;
  } finally {
    await socket.disconnect();
  }
}
