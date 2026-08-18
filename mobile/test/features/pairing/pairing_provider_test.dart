import 'dart:async';
import 'dart:convert';

import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:local_auth/local_auth.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:buzz/features/pairing/pairing_crypto.dart';
import 'package:buzz/features/pairing/pairing_provider.dart';
import 'package:buzz/features/pairing/pairing_socket.dart';
import 'package:buzz/shared/auth/auth.dart';
import 'package:buzz/shared/crypto/ecdh.dart';
import 'package:buzz/shared/crypto/nip44.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/security/sensitive_action_authorizer.dart';

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

    group('identity import protection', () {
      const sourceSecret =
          '09b3065e3570a3a4054660dccd66e12774a99a904fdb0ca02dbc6c3136249506';
      const sessionSecretHex =
          'abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789';
      late _ControllableSocket socket;
      late PairingNotifier notifier;
      late FakeAuthNotifier importAuth;
      late _FakeSensitiveActionAuthorizer authorizer;
      late Completer<void> validation;
      late String pairingCode;

      setUp(() {
        final source = nostr.Keys(sourceSecret);
        pairingCode =
            'nostrpair://${source.public}'
            '?secret=$sessionSecretHex'
            '&relay=wss%3A%2F%2Fpairing.buzz.xyz&v=1';
        validation = Completer<void>();
        importAuth = FakeAuthNotifier();
        authorizer = _FakeSensitiveActionAuthorizer();
        notifier = PairingNotifier(
          credentialValidator:
              ({required String relayUrl, required String? nsec}) =>
                  validation.future,
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
            authProvider.overrideWith(() => importAuth),
            sensitiveActionAuthorizerProvider.overrideWithValue(authorizer),
          ],
        );
        container.read(pairingProvider);
        notifier = container.read(pairingProvider.notifier);
      });

      Future<void> beginImport({required bool protected}) async {
        await notifier.pair(pairingCode);
        notifier.setProtectSensitiveActions(protected);
        notifier.confirmSas();
        socket.sendSourceMessage(
          sourceSecret: sourceSecret,
          sessionSecretHex: sessionSecretHex,
          message: {'type': 'sas-confirm'},
          includeTranscriptHash: true,
        );
        await Future<void>.delayed(Duration.zero);
        if (protected && authorizer.result != DeviceAuthResult.success) return;
        socket.sendSourceMessage(
          sourceSecret: sourceSecret,
          sessionSecretHex: sessionSecretHex,
          message: {
            'type': 'payload',
            'payload_type': 'credentials',
            'payload': jsonEncode({
              'relayUrl': 'https://relay.test',
              'pubkey': nostr.Keys(sourceSecret).public,
              'nsec': nostr.Keys(sourceSecret).nsec,
            }),
          },
        );
        await Future<void>.delayed(Duration.zero);
        expect(container.read(pairingProvider).status, PairingStatus.storing);
      }

      test('unchecked protection persists on a successful import', () async {
        await beginImport(protected: false);

        validation.complete();
        await Future<void>.delayed(Duration.zero);

        expect(
          importAuth.lastCommunity?.sensitiveActionPolicy,
          SensitiveActionPolicy.disabledByUser,
        );
        expect(container.read(pairingProvider).status, PairingStatus.success);
      });

      test('checked protection persists on a successful import', () async {
        await beginImport(protected: true);

        expect(authorizer.calls, 1);
        validation.complete();
        await Future<void>.delayed(Duration.zero);

        expect(
          importAuth.lastCommunity?.sensitiveActionPolicy,
          SensitiveActionPolicy.enabled,
        );
      });

      test('unenrolled biometrics prevent a protected import', () async {
        authorizer.result = DeviceAuthResult.unavailable;

        await beginImport(protected: true);

        final state = container.read(pairingProvider);
        expect(authorizer.calls, 1);
        expect(state.status, PairingStatus.confirmingSas);
        expect(state.userConfirmedSas, isFalse);
        expect(state.errorMessage, contains('Enroll Face ID or biometrics'));
        expect(importAuth.lastCommunity, isNull);
        expect(
          socket
              .decryptedPublishedMessages(sourceSecret)
              .any((message) => message['type'] == 'complete'),
          isFalse,
        );
      });

      test('stale biometric approval cannot advance a reset import', () async {
        final pendingAuthorization = Completer<DeviceAuthResult>();
        authorizer.pending = pendingAuthorization;
        await notifier.pair(pairingCode);
        notifier.setProtectSensitiveActions(true);
        notifier.confirmSas();
        socket.sendSourceMessage(
          sourceSecret: sourceSecret,
          sessionSecretHex: sessionSecretHex,
          message: {'type': 'sas-confirm'},
          includeTranscriptHash: true,
        );
        await Future<void>.delayed(Duration.zero);
        expect(authorizer.calls, 1);

        notifier.reset();
        pendingAuthorization.complete(DeviceAuthResult.success);
        await Future<void>.delayed(Duration.zero);

        expect(container.read(pairingProvider).status, PairingStatus.idle);
        expect(importAuth.lastCommunity, isNull);
        expect(
          socket
              .decryptedPublishedMessages(sourceSecret)
              .any((message) => message['type'] == 'complete'),
          isFalse,
        );
      });

      test('reset invalidates pending authentication persistence', () async {
        importAuth.authentication = Completer<void>();
        await beginImport(protected: false);

        validation.complete();
        await Future<void>.delayed(Duration.zero);
        expect(importAuth.lastCommunity, isNotNull);

        notifier.reset();
        importAuth.authentication!.complete();
        await Future<void>.delayed(Duration.zero);

        expect(container.read(pairingProvider).status, PairingStatus.idle);
        expect(socket.isConnected, isFalse);
      });

      test('reset invalidates pending credential validation', () async {
        await beginImport(protected: false);

        notifier.reset();
        validation.complete();
        await Future<void>.delayed(Duration.zero);

        expect(importAuth.lastCommunity, isNull);
        expect(container.read(pairingProvider).status, PairingStatus.idle);
        expect(
          socket
              .decryptedPublishedMessages(sourceSecret)
              .any((message) => message['type'] == 'complete'),
          isFalse,
        );
      });
    });

    group('desktop identity recovery', () {
      const sourceSecret =
          '09b3065e3570a3a4054660dccd66e12774a99a904fdb0ca02dbc6c3136249506';
      const sessionSecretHex =
          'abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789';
      late _ControllableSocket socket;
      late PairingNotifier notifier;
      late String recoveryCode;
      late _FakeSensitiveActionAuthorizer authorizer;
      late DateTime now;

      setUp(() async {
        final source = nostr.Keys(sourceSecret);
        recoveryCode =
            'nostrpair://${source.public}'
            '?secret=$sessionSecretHex'
            '&relay=wss%3A%2F%2Fpairing.buzz.xyz&v=1&mode=recover';
        authorizer = _FakeSensitiveActionAuthorizer();
        now = DateTime.utc(2026, 8, 6);
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
            sensitiveActionAuthorizerProvider.overrideWithValue(authorizer),
            identityExportClockProvider.overrideWithValue(() => now),
          ],
        );
        container.read(pairingProvider);
        notifier = container.read(pairingProvider.notifier);
      });

      test('recovery authorization honors biometric protection', () async {
        expect(
          await notifier.authorizeIdentityExport(
            community: _exportCommunity(SensitiveActionPolicy.enabled),
          ),
          isTrue,
        );
        expect(authorizer.calls, 1);
        expect(authorizer.lastIdentityBiometricOnly, isTrue);

        await notifier.pair(recoveryCode);

        final state = container.read(pairingProvider);
        expect(state.status, PairingStatus.confirmingSas);
        expect(state.sendsIdentityToDesktop, isTrue);
        expect(state.sasCode, hasLength(6));
      });

      test('reset invalidates pending preflight authorization', () async {
        final pendingAuthorization = Completer<DeviceAuthResult>();
        authorizer.pending = pendingAuthorization;

        final authorization = notifier.authorizeIdentityExport(
          community: _exportCommunity(SensitiveActionPolicy.disabledByUser),
        );
        expect(container.read(pairingProvider).authorizationInProgress, isTrue);

        notifier.reset();
        pendingAuthorization.complete(DeviceAuthResult.success);

        expect(await authorization, isFalse);
        expect(container.read(pairingProvider).status, PairingStatus.idle);
        await notifier.pair(recoveryCode);
        notifier.confirmSas();
        socket.sendSourceMessage(
          sourceSecret: sourceSecret,
          sessionSecretHex: sessionSecretHex,
          message: {'type': 'sas-confirm'},
          includeTranscriptHash: true,
        );
        await Future<void>.delayed(Duration.zero);
        expect(
          socket
              .decryptedPublishedMessages(sourceSecret)
              .any((message) => message['type'] == 'payload'),
          isFalse,
        );
      });

      test('concurrent preflight requests start one authentication', () async {
        final pendingAuthorization = Completer<DeviceAuthResult>();
        authorizer.pending = pendingAuthorization;
        final community = _exportCommunity(
          SensitiveActionPolicy.disabledByUser,
        );

        final first = notifier.authorizeIdentityExport(community: community);
        final second = notifier.authorizeIdentityExport(community: community);

        expect(await second, isFalse);
        expect(authorizer.calls, 1);
        pendingAuthorization.complete(DeviceAuthResult.success);
        expect(await first, isTrue);
      });

      test('unchecked protection allows device passcode fallback', () async {
        expect(
          await notifier.authorizeIdentityExport(
            community: _exportCommunity(SensitiveActionPolicy.disabledByUser),
          ),
          isTrue,
        );

        expect(authorizer.lastIdentityBiometricOnly, isFalse);
      });

      test(
        'matching SAS sends nsec and successful completion finishes',
        () async {
          expect(
            await notifier.authorizeIdentityExport(
              community: _exportCommunity(SensitiveActionPolicy.disabledByUser),
            ),
            isTrue,
          );
          await notifier.pair(recoveryCode);
          notifier.confirmSas();
          expect(container.read(pairingProvider).userConfirmedSas, isTrue);

          socket.sendSourceMessage(
            sourceSecret: sourceSecret,
            sessionSecretHex: sessionSecretHex,
            message: {'type': 'sas-confirm'},
            includeTranscriptHash: true,
          );

          await Future<void>.delayed(Duration.zero);

          expect(
            container.read(pairingProvider).status,
            PairingStatus.transferring,
          );
          expect(authorizer.calls, 1);
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

      test(
        'malformed payload invalidates authorization before another session',
        () async {
          expect(
            await notifier.authorizeIdentityExport(
              community: _exportCommunity(SensitiveActionPolicy.disabledByUser),
            ),
            isTrue,
          );
          await notifier.pair(recoveryCode);
          notifier.confirmSas();
          socket.sendSourceMessage(
            sourceSecret: sourceSecret,
            sessionSecretHex: sessionSecretHex,
            message: {'type': 'sas-confirm'},
            includeTranscriptHash: true,
          );
          await Future<void>.delayed(Duration.zero);

          socket.sendSourceMessage(
            sourceSecret: sourceSecret,
            sessionSecretHex: sessionSecretHex,
            message: {'type': 'payload', 'payload_type': 'nsec'},
          );
          expect(container.read(pairingProvider).status, PairingStatus.error);

          await notifier.pair(recoveryCode);
          final replacementSocket = socket;
          notifier.confirmSas();
          replacementSocket.sendSourceMessage(
            sourceSecret: sourceSecret,
            sessionSecretHex: sessionSecretHex,
            message: {'type': 'sas-confirm'},
            includeTranscriptHash: true,
          );
          await Future<void>.delayed(Duration.zero);

          final state = container.read(pairingProvider);
          expect(state.status, PairingStatus.confirmingSas);
          expect(state.userConfirmedSas, isFalse);
          expect(state.errorMessage, contains('active community changed'));
          expect(authorizer.calls, 1);
          expect(
            replacementSocket
                .decryptedPublishedMessages(sourceSecret)
                .any((message) => message['type'] == 'payload'),
            isFalse,
          );
        },
      );

      test('skipped preflight authorization sends no identity', () async {
        await notifier.pair(recoveryCode);
        notifier.confirmSas();
        socket.sendSourceMessage(
          sourceSecret: sourceSecret,
          sessionSecretHex: sessionSecretHex,
          message: {'type': 'sas-confirm'},
          includeTranscriptHash: true,
        );
        await Future<void>.delayed(Duration.zero);

        final state = container.read(pairingProvider);
        expect(authorizer.calls, 0);
        expect(state.status, PairingStatus.confirmingSas);
        expect(state.userConfirmedSas, isFalse);
        expect(state.errorMessage, contains('active community changed'));
        expect(
          socket
              .decryptedPublishedMessages(sourceSecret)
              .any((message) => message['type'] == 'payload'),
          isFalse,
        );
      });

      test(
        'failed transfer reauthorization publishes no identity payload',
        () async {
          expect(
            await notifier.authorizeIdentityExport(
              community: _exportCommunity(SensitiveActionPolicy.disabledByUser),
            ),
            isTrue,
          );
          await notifier.pair(recoveryCode);
          now = now.add(identityExportAuthorizationTtl);
          authorizer.result = DeviceAuthResult.cancelled;
          notifier.confirmSas();
          socket.sendSourceMessage(
            sourceSecret: sourceSecret,
            sessionSecretHex: sessionSecretHex,
            message: {'type': 'sas-confirm'},
            includeTranscriptHash: true,
          );
          await Future<void>.delayed(Duration.zero);

          expect(authorizer.calls, 2);
          final state = container.read(pairingProvider);
          expect(state.status, PairingStatus.confirmingSas);
          expect(state.userConfirmedSas, isFalse);
          expect(state.errorMessage, contains('cancelled'));
          final sentMessages = socket.decryptedPublishedMessages(sourceSecret);
          expect(
            sentMessages.any((message) => message['type'] == 'payload'),
            isFalse,
          );
        },
      );

      test(
        'stale authorization cannot advance a replacement pairing session',
        () async {
          expect(
            await notifier.authorizeIdentityExport(
              community: _exportCommunity(SensitiveActionPolicy.disabledByUser),
            ),
            isTrue,
          );
          await notifier.pair(recoveryCode);
          now = now.add(identityExportAuthorizationTtl);
          final firstSocket = socket;
          final pendingAuthorization = Completer<DeviceAuthResult>();
          authorizer.pending = pendingAuthorization;
          notifier.confirmSas();
          firstSocket.sendSourceMessage(
            sourceSecret: sourceSecret,
            sessionSecretHex: sessionSecretHex,
            message: {'type': 'sas-confirm'},
            includeTranscriptHash: true,
          );
          await Future<void>.delayed(Duration.zero);
          expect(authorizer.calls, 2);

          notifier.reset();
          await notifier.pair(recoveryCode);
          final replacementSocket = socket;
          expect(
            container.read(pairingProvider).status,
            PairingStatus.confirmingSas,
          );
          expect(container.read(pairingProvider).userConfirmedSas, isFalse);

          pendingAuthorization.complete(DeviceAuthResult.success);
          await Future<void>.delayed(Duration.zero);

          final state = container.read(pairingProvider);
          expect(state.status, PairingStatus.confirmingSas);
          expect(state.userConfirmedSas, isFalse);
          for (final pairingSocket in [firstSocket, replacementSocket]) {
            final sentMessages = pairingSocket.decryptedPublishedMessages(
              sourceSecret,
            );
            expect(
              sentMessages.any((message) => message['type'] == 'payload'),
              isFalse,
            );
          }
        },
      );

      test('clock rollback invalidates the export authorization', () async {
        expect(
          await notifier.authorizeIdentityExport(
            community: _exportCommunity(SensitiveActionPolicy.disabledByUser),
          ),
          isTrue,
        );
        await notifier.pair(recoveryCode);
        now = now.subtract(const Duration(minutes: 1));
        notifier.confirmSas();
        socket.sendSourceMessage(
          sourceSecret: sourceSecret,
          sessionSecretHex: sessionSecretHex,
          message: {'type': 'sas-confirm'},
          includeTranscriptHash: true,
        );
        await Future<void>.delayed(Duration.zero);

        expect(authorizer.calls, 2);
        expect(
          container.read(pairingProvider).status,
          PairingStatus.transferring,
        );
      });

      test(
        'websocket relay origin remains bound after canonicalization',
        () async {
          final community = _exportCommunity(
            SensitiveActionPolicy.disabledByUser,
          ).copyWith(relayUrl: 'wss://relay.test');
          container
              .read(relayConfigProvider.notifier)
              .update(baseUrl: community.relayUrl, nsec: community.nsec);
          expect(
            await notifier.authorizeIdentityExport(community: community),
            isTrue,
          );
          await notifier.pair(recoveryCode);
          notifier.confirmSas();
          socket.sendSourceMessage(
            sourceSecret: sourceSecret,
            sessionSecretHex: sessionSecretHex,
            message: {'type': 'sas-confirm'},
            includeTranscriptHash: true,
          );
          await Future<void>.delayed(Duration.zero);

          expect(
            container.read(pairingProvider).status,
            PairingStatus.transferring,
          );
          expect(
            socket
                .decryptedPublishedMessages(sourceSecret)
                .any(
                  (message) =>
                      message['type'] == 'payload' &&
                      message['payload'] == community.nsec,
                ),
            isTrue,
          );
        },
      );

      for (final expired in [false, true]) {
        test(
          'community switch aborts ${expired ? 'expired' : 'fresh'} export grant',
          () async {
            expect(
              await notifier.authorizeIdentityExport(
                community: _exportCommunity(
                  SensitiveActionPolicy.disabledByUser,
                ),
              ),
              isTrue,
            );
            await notifier.pair(recoveryCode);
            if (expired) now = now.add(identityExportAuthorizationTtl);
            container
                .read(relayConfigProvider.notifier)
                .update(
                  baseUrl: 'https://other-relay.test',
                  nsec: nostr.Keys(
                    '2222222222222222222222222222222222222222222222222222222222222222',
                  ).nsec,
                );
            notifier.confirmSas();
            socket.sendSourceMessage(
              sourceSecret: sourceSecret,
              sessionSecretHex: sessionSecretHex,
              message: {'type': 'sas-confirm'},
              includeTranscriptHash: true,
            );
            await Future<void>.delayed(Duration.zero);

            final state = container.read(pairingProvider);
            expect(state.status, PairingStatus.confirmingSas);
            expect(state.userConfirmedSas, isFalse);
            expect(state.errorMessage, contains('active community changed'));
            expect(authorizer.calls, 1);
            expect(
              socket
                  .decryptedPublishedMessages(sourceSecret)
                  .any((message) => message['type'] == 'payload'),
              isFalse,
            );
          },
        );
      }

      test('expired biometric-only authorization preserves its mode', () async {
        expect(
          await notifier.authorizeIdentityExport(
            community: _exportCommunity(SensitiveActionPolicy.enabled),
          ),
          isTrue,
        );
        await notifier.pair(recoveryCode);
        now = now.add(identityExportAuthorizationTtl);
        notifier.confirmSas();
        socket.sendSourceMessage(
          sourceSecret: sourceSecret,
          sessionSecretHex: sessionSecretHex,
          message: {'type': 'sas-confirm'},
          includeTranscriptHash: true,
        );
        await Future<void>.delayed(Duration.zero);

        expect(authorizer.calls, 2);
        expect(authorizer.lastIdentityBiometricOnly, isTrue);
        expect(
          container.read(pairingProvider).status,
          PairingStatus.transferring,
        );
      });

      test(
        'expired passcode-fallback authorization preserves its mode',
        () async {
          expect(
            await notifier.authorizeIdentityExport(
              community: _exportCommunity(SensitiveActionPolicy.disabledByUser),
            ),
            isTrue,
          );
          await notifier.pair(recoveryCode);
          now = now.add(identityExportAuthorizationTtl);
          notifier.confirmSas();
          socket.sendSourceMessage(
            sourceSecret: sourceSecret,
            sessionSecretHex: sessionSecretHex,
            message: {'type': 'sas-confirm'},
            includeTranscriptHash: true,
          );
          await Future<void>.delayed(Duration.zero);

          expect(authorizer.calls, 2);
          expect(authorizer.lastIdentityBiometricOnly, isFalse);
          expect(
            container.read(pairingProvider).status,
            PairingStatus.transferring,
          );
        },
      );

      test('cancelled authorization prevents pairing from starting', () async {
        authorizer.result = DeviceAuthResult.cancelled;

        expect(
          await notifier.authorizeIdentityExport(
            community: _exportCommunity(SensitiveActionPolicy.disabledByUser),
          ),
          isFalse,
        );

        expect(authorizer.calls, 1);
        expect(container.read(pairingProvider).status, PairingStatus.idle);
        expect(
          container.read(pairingProvider).errorMessage,
          contains('cancelled'),
        );
      });

      test('desktop storage failure surfaces an error', () async {
        expect(
          await notifier.authorizeIdentityExport(
            community: _exportCommunity(SensitiveActionPolicy.disabledByUser),
          ),
          isTrue,
        );
        await notifier.pair(recoveryCode);
        notifier.confirmSas();
        socket.sendSourceMessage(
          sourceSecret: sourceSecret,
          sessionSecretHex: sessionSecretHex,
          message: {'type': 'sas-confirm'},
          includeTranscriptHash: true,
        );
        await Future<void>.delayed(Duration.zero);
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
  Completer<void>? authentication;
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
    await authentication?.future;
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

Community _exportCommunity(SensitiveActionPolicy policy) => Community(
  id: 'export-community',
  name: 'Export',
  relayUrl: 'https://relay.test',
  nsec: _RecoveryRelayConfig.nsec,
  sensitiveActionPolicy: policy,
  addedAt: DateTime.utc(2026),
);

class _RecoveryRelayConfig extends RelayConfigNotifier {
  static final nsec = nostr.Keys(
    '1111111111111111111111111111111111111111111111111111111111111111',
  ).nsec;

  @override
  RelayConfig build() => RelayConfig(baseUrl: 'https://relay.test', nsec: nsec);
}

class _FakeSensitiveActionAuthorizer implements SensitiveActionAuthorizer {
  DeviceAuthResult result = DeviceAuthResult.success;
  Completer<DeviceAuthResult>? pending;
  int calls = 0;
  bool? lastIdentityBiometricOnly;

  @override
  Future<DeviceAuthResult> authorizeIdentityAction({
    required bool biometricOnly,
  }) async {
    calls++;
    lastIdentityBiometricOnly = biometricOnly;
    return pending?.future ?? result;
  }

  @override
  Future<DeviceAuthResult> authorizeBiometricProtection() async {
    calls++;
    return pending?.future ?? result;
  }

  @override
  Future<List<BiometricType>> enrolledBiometrics() async => const [];
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
