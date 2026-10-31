import 'dart:convert';
import 'package:nostr/nostr.dart' as nostr;
import 'package:buzz/features/pairing/pairing_crypto.dart';
import 'package:buzz/features/pairing/pairing_socket.dart';
import 'package:buzz/shared/crypto/ecdh.dart';
import 'package:buzz/shared/crypto/nip44.dart';

class ControllablePairingSocket extends PairingSocket {
  final String ephemeralPrivkey;
  final void Function(List<dynamic> message) relayMessageCallback;
  final List<Map<String, dynamic>> published = [];
  bool _connected = false;
  int _eventSequence = 0;

  ControllablePairingSocket({
    required this.ephemeralPrivkey,
    required super.onMessage,
    required super.onDisconnected,
  }) : relayMessageCallback = onMessage,
       super(wsUrl: 'ws://unused', ephemeralPrivkey: ephemeralPrivkey);

  @override
  bool get isConnected => _connected;

  @override
  Future<void> connect() async => _connected = true;

  @override
  void subscribe(String subId, int kind, String pubkeyHex) {}

  @override
  void publishEvent(Map<String, dynamic> event) => published.add(event);

  @override
  void dispose() => _connected = false;

  List<Map<String, dynamic>> decryptedPublishedMessages(String sourceSecret) {
    final key = getConversationKey(
      sourceSecret,
      nostr.Keys(ephemeralPrivkey).public,
    );
    return published
        .map(
          (event) =>
              jsonDecode(nip44Decrypt(key, event['content'] as String))
                  as Map<String, dynamic>,
        )
        .toList();
  }

  void sendSourceMessage({
    required String sourceSecret,
    required String sessionSecretHex,
    required Map<String, dynamic> message,
    bool includeTranscriptHash = false,
  }) {
    final source = nostr.Keys(sourceSecret);
    final targetPubkey = nostr.Keys(ephemeralPrivkey).public;
    final sessionSecret = hexToBytes(sessionSecretHex);
    final body = Map<String, dynamic>.from(message);
    if (includeTranscriptHash) {
      final shared = ecdhSharedSecret(sourceSecret, targetPubkey);
      final (_, sasInput) = deriveSas(shared, sessionSecret);
      body['transcript_hash'] = bytesToHex(
        deriveTranscriptHash(
          deriveSessionId(sessionSecret),
          hexToBytes(source.public),
          hexToBytes(targetPubkey),
          sasInput,
          sessionSecret,
        ),
      );
    }
    final key = getConversationKey(sourceSecret, targetPubkey);
    final event = nostr.Event.from(
      kind: 24134,
      content: nip44Encrypt(key, jsonEncode(body)),
      tags: [
        ['p', targetPubkey],
      ],
      secretKey: sourceSecret,
      createdAt: 1_700_000_000 + _eventSequence++,
    );
    relayMessageCallback(['EVENT', 'pair', event.toMap()]);
  }
}
