import 'dart:convert';
import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:buzz/features/pairing/pairing_crypto.dart';
import 'package:buzz/features/pairing/pairing_provider.dart';
import 'package:buzz/features/pairing/pairing_socket.dart';
import 'package:buzz/shared/auth/auth.dart';
import 'package:buzz/shared/crypto/ecdh.dart';
import 'package:buzz/shared/crypto/nip44.dart';

/// Tests for [PairingNotifier]'s NIP-AB flow, legacy `buzz://` parsing,
/// and SSRF-prevention validation.
///
/// The credential validator is injected for the NIP-AB end-to-end test so it
/// exercises signed event validation, NIP-44 decryption, out-of-order event
/// buffering, explicit SAS approval, credential handoff, and completion
/// without contacting an external relay.
void main() {
  group('PairingNotifier', () {
    late ProviderContainer container;
    late FakeAuthNotifier fakeAuth;

    ProviderContainer createContainer() {
      fakeAuth = FakeAuthNotifier();
      return ProviderContainer(
        overrides: [authProvider.overrideWith(() => fakeAuth)],
      );
    }

    tearDown(() => container.dispose());

    test('starts in idle state', () {
      container = createContainer();
      final state = container.read(pairingProvider);
      expect(state.status, PairingStatus.idle);
      expect(state.errorMessage, isNull);
    });

    test(
      'disconnect during connect does not null-dereference the socket',
      () async {
        final notifier = PairingNotifier(
          socketFactory:
              ({
                required wsUrl,
                required ephemeralPrivkey,
                required onMessage,
                required void Function(Object? error) onDisconnected,
              }) => _DisconnectingSocket(disconnectCallback: onDisconnected),
        );
        container = ProviderContainer(
          overrides: [pairingProvider.overrideWith(() => notifier)],
        );
        const code =
            'nostrpair://62287897da61e3fa294b4570575f7db8bea147d6631150f2e4656714c645fb1e'
            '?secret=abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789'
            '&relay=wss%3A%2F%2Fpairing.buzz.xyz&v=1';

        await container.read(pairingProvider.notifier).pair(code);

        expect(container.read(pairingProvider).status, PairingStatus.error);
        expect(
          container.read(pairingProvider).errorMessage,
          contains('internal error'),
        );
      },
    );

    test('payload missing nsec errors before contacting relay', () async {
      container = createContainer();

      // Valid payload shape but no nsec — provider should refuse without
      // attempting any network call.
      final code = _encodePairingCode();
      await container.read(pairingProvider.notifier).pair(code);

      final state = container.read(pairingProvider);
      expect(state.status, PairingStatus.error);
      expect(state.errorMessage, contains('missing nsec'));
      expect(fakeAuth.lastCommunity, isNull);
    });

    test('accepts buzz scheme prefix', () async {
      container = createContainer();

      final code = 'buzz://${_encodePairingCode()}';
      await container.read(pairingProvider.notifier).pair(code);

      final state = container.read(pairingProvider);
      expect(state.status, PairingStatus.error);
      expect(state.errorMessage, contains('missing nsec'));
      expect(fakeAuth.lastCommunity, isNull);
    });

    test('invalid base64 sets format error', () async {
      container = createContainer();

      await container.read(pairingProvider.notifier).pair('not-valid!!!');

      final state = container.read(pairingProvider);
      expect(state.status, PairingStatus.error);
      expect(state.errorMessage, contains('Invalid pairing code'));
    });

    test('base64 with valid JSON but missing fields errors', () async {
      container = createContainer();

      final code = base64Url.encode(utf8.encode(jsonEncode({'foo': 'bar'})));
      await container.read(pairingProvider.notifier).pair(code);

      final state = container.read(pairingProvider);
      expect(state.status, PairingStatus.error);
      expect(state.errorMessage, contains('Missing relayUrl'));
    });

    test('empty input errors', () async {
      container = createContainer();

      await container.read(pairingProvider.notifier).pair('');

      final state = container.read(pairingProvider);
      expect(state.status, PairingStatus.error);
    });

    test('rejects private IP relay URLs (SSRF)', () async {
      container = createContainer();

      for (final ip in [
        '10.0.0.1',
        '172.16.0.1',
        '192.168.1.1',
        '169.254.169.254',
      ]) {
        final code = _encodePairingCode(relayUrl: 'http://$ip:3000');
        await container.read(pairingProvider.notifier).pair(code);
        final state = container.read(pairingProvider);
        expect(state.status, PairingStatus.error, reason: 'should reject $ip');
        expect(state.errorMessage, contains('private network'));
        container.read(pairingProvider.notifier).reset();
      }
    });

    test('rejects non-http/https schemes', () async {
      container = createContainer();

      final code = _encodePairingCode(relayUrl: 'file:///etc/passwd');
      await container.read(pairingProvider.notifier).pair(code);

      final state = container.read(pairingProvider);
      expect(state.status, PairingStatus.error);
      expect(state.errorMessage, contains('Invalid pairing code'));
    });

    test('rejects JSON array payload', () async {
      container = createContainer();

      final code = base64Url.encode(utf8.encode(jsonEncode([1, 2, 3])));
      await container.read(pairingProvider.notifier).pair(code);

      final state = container.read(pairingProvider);
      expect(state.status, PairingStatus.error);
      expect(state.errorMessage, contains('not a JSON object'));
    });

    test(
      'payload delivered before confirmation is buffered until user approval',
      () async {
        final sourceKeys = nostr.Keys('01' * 32);
        final sessionSecret = Uint8List.fromList(
          List<int>.generate(32, (index) => index + 1),
        );
        _RecordingPairingSocket? createdPairingSocket;
        String? validatedRelayUrl;
        String? validatedNsec;
        final notifier = PairingNotifier(
          socketFactory:
              ({
                required wsUrl,
                required ephemeralPrivkey,
                required onMessage,
                required onDisconnected,
              }) {
                createdPairingSocket = _RecordingPairingSocket(
                  ephemeralPrivkey: ephemeralPrivkey,
                  onMessage: onMessage,
                );
                return createdPairingSocket!;
              },
          credentialValidator: ({required relayUrl, required nsec}) async {
            validatedRelayUrl = relayUrl;
            validatedNsec = nsec;
          },
        );
        fakeAuth = FakeAuthNotifier();
        container = ProviderContainer(
          overrides: [
            authProvider.overrideWith(() => fakeAuth),
            pairingProvider.overrideWith(() => notifier),
          ],
        );
        final code =
            'nostrpair://${sourceKeys.public}'
            '?secret=${bytesToHex(sessionSecret)}'
            '&relay=wss%3A%2F%2Fpairing.example.test&v=1';

        await container.read(pairingProvider.notifier).pair(code);
        expect(
          container.read(pairingProvider).status,
          PairingStatus.confirmingSas,
        );
        final pairingSocket = createdPairingSocket!;

        final targetPubkey = nostr.Keys(pairingSocket.ephemeralPrivkey).public;
        final ecdhShared = ecdhSharedSecret(sourceKeys.secret, targetPubkey);
        final (_, sasInput) = deriveSas(ecdhShared, sessionSecret);
        final transcriptHash = deriveTranscriptHash(
          deriveSessionId(sessionSecret),
          hexToBytes(sourceKeys.public),
          hexToBytes(targetPubkey),
          sasInput,
          sessionSecret,
        );
        final conversationKey = getConversationKey(
          sourceKeys.secret,
          targetPubkey,
        );
        Map<String, dynamic> sourceEvent(Map<String, dynamic> message) =>
            nostr.Event.from(
              kind: 24134,
              content: nip44Encrypt(conversationKey, jsonEncode(message)),
              tags: [
                ['p', targetPubkey],
              ],
              secretKey: sourceKeys.secret,
              createdAt: DateTime.now().millisecondsSinceEpoch ~/ 1000,
            ).toMap();

        // Relays do not guarantee ordering between back-to-back events. Deliver
        // the payload first to reproduce the production race.
        pairingSocket.emit(
          sourceEvent({
            'type': 'payload',
            'payload_type': 'custom',
            'payload': jsonEncode({
              'relayUrl': 'https://relay.example.test',
              'pubkey': sourceKeys.public,
              'nsec': sourceKeys.nsec,
            }),
          }),
        );
        expect(validatedRelayUrl, isNull);
        expect(fakeAuth.lastCommunity, isNull);
        expect(
          container.read(pairingProvider).status,
          PairingStatus.confirmingSas,
        );
        pairingSocket.emit(
          sourceEvent({
            'type': 'sas-confirm',
            'transcript_hash': bytesToHex(transcriptHash),
          }),
        );
        expect(validatedRelayUrl, isNull);
        expect(fakeAuth.lastCommunity, isNull);
        expect(
          container.read(pairingProvider).status,
          PairingStatus.confirmingSas,
        );

        container.read(pairingProvider.notifier).confirmSas();
        await pumpEventQueue(times: 20);

        expect(validatedRelayUrl, 'https://relay.example.test');
        expect(validatedNsec, sourceKeys.nsec);
        expect(fakeAuth.lastCommunity?.pubkey, sourceKeys.public);
        expect(container.read(pairingProvider).status, PairingStatus.success);
        expect(pairingSocket.publishedEvents, isNotEmpty);
      },
    );

    test('reset returns to idle from error state', () async {
      container = createContainer();

      // Trigger an error.
      await container.read(pairingProvider.notifier).pair('not-valid!!!');
      expect(container.read(pairingProvider).status, PairingStatus.error);

      container.read(pairingProvider.notifier).reset();
      expect(container.read(pairingProvider).status, PairingStatus.idle);
    });
  });
}

/// Encode a credentials payload the same way the desktop app would.
String _encodePairingCode({
  String relayUrl = 'http://test:3000',
  String? pubkey,
  String? nsec,
}) {
  final json = <String, dynamic>{
    'relayUrl': relayUrl,
    // ignore: use_null_aware_elements
    if (pubkey != null) 'pubkey': pubkey,
    // ignore: use_null_aware_elements
    if (nsec != null) 'nsec': nsec,
  };
  return base64Url.encode(utf8.encode(jsonEncode(json)));
}

/// A fake [AuthNotifier] that records calls instead of touching secure storage.
class FakeAuthNotifier extends AsyncNotifier<AuthState>
    implements AuthNotifier {
  Community? lastCommunity;
  bool signedOut = false;

  @override
  Future<AuthState> build() async =>
      const AuthState(status: AuthStatus.unauthenticated);

  @override
  Future<void> signOut() async {
    signedOut = true;
    state = const AsyncData(AuthState(status: AuthStatus.unauthenticated));
  }

  @override
  Future<void> authenticateWithCommunity(Community community) async {
    lastCommunity = community;
    state = AsyncData(
      AuthState(status: AuthStatus.authenticated, community: community),
    );
  }
}

class _RecordingPairingSocket extends PairingSocket {
  final String ephemeralPrivkey;
  final void Function(List<dynamic> message) messageCallback;
  final List<Map<String, dynamic>> publishedEvents = [];
  bool _isConnected = false;

  _RecordingPairingSocket({
    required this.ephemeralPrivkey,
    required void Function(List<dynamic> message) onMessage,
  }) : messageCallback = onMessage,
       super(
         wsUrl: 'ws://unused',
         ephemeralPrivkey: ephemeralPrivkey,
         onMessage: (_) {},
         onDisconnected: (_) {},
       );

  @override
  bool get isConnected => _isConnected;

  @override
  Future<void> connect() async {
    _isConnected = true;
  }

  @override
  void subscribe(String subId, int kind, String pubkeyHex) {}

  @override
  void publishEvent(Map<String, dynamic> event) {
    publishedEvents.add(event);
  }

  void emit(Map<String, dynamic> event) {
    messageCallback(['EVENT', 'pair', event]);
  }

  @override
  void dispose() {
    _isConnected = false;
  }
}

class _DisconnectingSocket extends PairingSocket {
  final void Function(Object? error) disconnectCallback;

  _DisconnectingSocket({required this.disconnectCallback})
    : super(
        wsUrl: 'ws://unused',
        ephemeralPrivkey:
            '09b3065e3570a3a4054660dccd66e12774a99a904fdb0ca02dbc6c3136249506',
        onMessage: (_) {},
        onDisconnected: (_) {},
      );

  @override
  Future<void> connect() async {
    disconnectCallback(Exception('Connection closed'));
  }
}
