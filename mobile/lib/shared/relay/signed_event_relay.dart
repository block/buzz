import 'dart:async';

import 'package:nostr/nostr.dart' as nostr;

import '../auth/enterprise_identity.dart';
import 'nostr_models.dart';
import 'relay_session.dart';
import 'relay_socket.dart';

/// Signs and submits Nostr events through the relay WebSocket connection.
class SignedEventRelay {
  final RelaySessionNotifier _session;
  final String? _nsec;
  final String? _corporatePubkey;

  SignedEventRelay({
    required RelaySessionNotifier session,
    required String? nsec,
  }) : _corporatePubkey = enterpriseEnabled
           ? EnterpriseIdentity.instance.pubkey
           : null,
       _session = session,
       _nsec = nsec;

  /// The hex pubkey derived from the signing key, or null if no key.
  String? get pubkey {
    if (enterpriseEnabled) return _corporatePubkey;
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
    if (enterpriseEnabled &&
        (_corporatePubkey == null ||
            _corporatePubkey != EnterpriseIdentity.instance.pubkey)) {
      throw StateError('Corporate identity changed before signing');
    }
    final event = await signClientEvent(
      nsec: _nsec,
      kind: kind,
      content: content,
      tags: tags,
      createdAt: createdAt,
    );

    if (enterpriseEnabled &&
        _corporatePubkey != EnterpriseIdentity.instance.pubkey) {
      throw StateError('Corporate identity changed before publication');
    }
    final nostrEvent = NostrEvent.fromJson(event.toMap());
    onSigned?.call(nostrEvent);
    return _session.publish(nostrEvent);
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
  if (enterpriseEnabled &&
      wsUrl !=
          Uri.parse(
            EnterpriseIdentity.instance.relayUrl!,
          ).replace(scheme: 'wss').toString()) {
    throw StateError('Corporate community mismatch');
  }
  final signed = await signClientEvent(
    nsec: nsec,
    kind: kind,
    content: content,
    tags: tags,
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
