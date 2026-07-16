import 'dart:convert';

import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:buzz/features/pairing/pairing_provider.dart';
import 'package:buzz/features/pairing/pairing_socket.dart';
import 'package:buzz/shared/auth/auth.dart';

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
