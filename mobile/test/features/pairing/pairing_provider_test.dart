import 'dart:convert';

import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:buzz/features/pairing/pairing_crypto.dart';
import 'package:buzz/features/pairing/pairing_provider.dart';
import 'package:buzz/features/pairing/pairing_socket.dart';
import 'package:buzz/shared/auth/auth.dart';
import 'package:buzz/shared/crypto/ecdh.dart';
import 'package:buzz/shared/crypto/nip44.dart';
import 'package:buzz/shared/relay/relay.dart';

/// Tests for [PairingNotifier]'s legacy `buzz://` payload parsing and
/// SSRF-prevention validation.
///
/// The pairing flow used to validate by calling `GET /api/users/me/profile`
/// over HTTP. That has been replaced with a NIP-42 WebSocket handshake via
/// [RelaySocket], which is constructed directly inside the provider with no
/// dependency-injection hook — so the "happy path" that exercises the
/// network is no longer mockable in a unit test.
///
/// What we still cover here:
///   - Initial state.
///   - Parsing every documented payload format (raw base64, `buzz://`
///     prefix, whitespace).
///   - Failure modes that return BEFORE any network call: invalid base64,
///     wrong shape (non-object, missing fields, missing nsec), and SSRF
///     guards (private IPs, non-http schemes).
///   - `reset()` returning to idle from an error state.
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

    test('reset returns to idle from error state', () async {
      container = createContainer();

      // Trigger an error.
      await container.read(pairingProvider.notifier).pair('not-valid!!!');
      expect(container.read(pairingProvider).status, PairingStatus.error);

      container.read(pairingProvider.notifier).reset();
      expect(container.read(pairingProvider).status, PairingStatus.idle);
    });

    group('desktop identity recovery', () {
      const sourceSecret =
          '09b3065e3570a3a4054660dccd66e12774a99a904fdb0ca02dbc6c3136249506';
      const sessionSecretHex =
          'abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789';
      late _ControllableSocket socket;
      late PairingNotifier notifier;
      late String recoveryCode;

      setUp(() {
        final source = nostr.Keys(sourceSecret);
        recoveryCode =
            'nostrpair://${source.public}'
            '?secret=$sessionSecretHex'
            '&relay=wss%3A%2F%2Fpairing.buzz.xyz&v=1&mode=recover';
        notifier = PairingNotifier(
          socketFactory:
              ({
                required wsUrl,
                required ephemeralPrivkey,
                required onMessage,
                required onDisconnected,
              }) {
                socket = _ControllableSocket(
                  ephemeralPrivkey: ephemeralPrivkey,
                  onMessage: onMessage,
                  onDisconnected: onDisconnected,
                );
                return socket;
              },
        );
        container = ProviderContainer(
          overrides: [
            pairingProvider.overrideWith(() => notifier),
            relayConfigProvider.overrideWith(_RecoveryRelayConfig.new),
          ],
        );
        container.read(pairingProvider);
        notifier = container.read(pairingProvider.notifier);
      });

      test('recovery URI enables phone-to-desktop transfer', () async {
        await notifier.pair(recoveryCode);

        final state = container.read(pairingProvider);
        expect(state.status, PairingStatus.confirmingSas);
        expect(state.sendsIdentityToDesktop, isTrue);
        expect(state.sasCode, hasLength(6));
      });

      test(
        'matching SAS sends nsec and successful completion finishes',
        () async {
          await notifier.pair(recoveryCode);
          notifier.confirmSas();
          expect(container.read(pairingProvider).userConfirmedSas, isTrue);

          socket.sendSourceMessage(
            sourceSecret: sourceSecret,
            sessionSecretHex: sessionSecretHex,
            message: {'type': 'sas-confirm'},
            includeTranscriptHash: true,
          );

          expect(
            container.read(pairingProvider).status,
            PairingStatus.transferring,
          );
          final sentMessages = socket.decryptedPublishedMessages(sourceSecret);
          expect(
            sentMessages.any(
              (message) =>
                  message['type'] == 'payload' &&
                  message['payload_type'] == 'nsec' &&
                  message['payload'] == _RecoveryRelayConfig.nsec,
            ),
            isTrue,
          );

          socket.sendSourceMessage(
            sourceSecret: sourceSecret,
            sessionSecretHex: sessionSecretHex,
            message: {'type': 'complete', 'success': true},
          );
          expect(container.read(pairingProvider).status, PairingStatus.success);
        },
      );

      test('desktop storage failure surfaces an error', () async {
        await notifier.pair(recoveryCode);
        notifier.confirmSas();
        socket.sendSourceMessage(
          sourceSecret: sourceSecret,
          sessionSecretHex: sessionSecretHex,
          message: {'type': 'sas-confirm'},
          includeTranscriptHash: true,
        );
        socket.sendSourceMessage(
          sourceSecret: sourceSecret,
          sessionSecretHex: sessionSecretHex,
          message: {'type': 'complete', 'success': false},
        );

        final state = container.read(pairingProvider);
        expect(state.status, PairingStatus.error);
        expect(state.errorMessage, contains('could not store'));
      });
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

class _RecoveryRelayConfig extends RelayConfigNotifier {
  static final nsec = nostr.Keys(
    '1111111111111111111111111111111111111111111111111111111111111111',
  ).nsec;

  @override
  RelayConfig build() => RelayConfig(baseUrl: 'https://relay.test', nsec: nsec);
}

class _ControllableSocket extends PairingSocket {
  final String ephemeralPrivkey;
  final void Function(List<dynamic> message) relayMessageCallback;
  final List<Map<String, dynamic>> published = [];
  bool _connected = false;
  int _eventSequence = 0;

  _ControllableSocket({
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
