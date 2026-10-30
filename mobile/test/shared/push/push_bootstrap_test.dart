import 'dart:convert';
import 'package:buzz/shared/push/dev_push_lease.dart';
import 'package:buzz/shared/community/community.dart';
import 'package:buzz/shared/community/community_provider.dart';
import 'package:buzz/shared/community/community_storage.dart';
import 'package:buzz/shared/push/push_bootstrap.dart';
import 'package:buzz/shared/push/push_bridge.dart';
import 'package:buzz/shared/push/push_subscription.dart';
import 'package:buzz/shared/push/push_subscription_provider.dart';
import 'package:buzz/shared/push/push_lease_revocation_outbox.dart';
import 'package:buzz/shared/push/push_relay_capability_provider.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:shared_preferences/shared_preferences.dart';
import '../community/community_storage_test.dart' show FakeSecureStorage;

void main() {
  testWidgets(
    'gateway opt-out retries independently while relay stays disconnected across restart',
    (tester) async {
      debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
      SharedPreferences.setMockInitialValues({});
      final prefs = await SharedPreferences.getInstance();
      final keys = nostr.Keys('1' * 64);
      final relayKey = nostr.Keys('2' * 64).public;
      final subscription = BuzzPushSubscription(
        filter: BuzzPushFilter(kinds: [9], pTags: [keys.public]),
        notificationClass: 'default',
      );
      final community =
          Community.create(
            name: 'Team',
            relayUrl: 'https://relay.example',
            nsec: keys.nsec,
            pubkey: keys.public,
          ).copyWith(
            pushSubscriptionState: BuzzPushLeaseSubscriptionState.desired(
              desired: [subscription],
            ).withAccepted(subscriptions: [subscription], generation: 7),
          );
      final storage = CommunityStorage(secure: FakeSecureStorage());
      await storage.save(community);
      final journal = BuzzPushLeaseRevocationStorage(
        secure: FakeSecureStorage(),
      );
      var grant = 'opaque-before-revoke';
      var calls = 0;
      var nativeReads = 0;
      var relayTombstones = 0;
      var sessionDisconnected = true;
      var failGateway = true;
      const native = MethodChannel('buzz/push');
      final messenger = tester.binding.defaultBinaryMessenger;
      messenger.setMockMethodCallHandler(native, (call) async {
        if (call.method == 'endpointGrants') {
          nativeReads++;
          return [
            {
              'relayOrigin': 'wss://relay.example',
              'relayPubkey': relayKey,
              'installationId': '0' * 32,
              'endpointGrant': grant,
              'endpointHash': 'b' * 64,
              'endpointEpoch': 1,
              'generation': 3,
              'expiresAt':
                  DateTime.now().millisecondsSinceEpoch ~/ 1000 + 86400,
              'gatewayInstallationHandle':
                  '123e4567-e89b-42d3-a456-426614174000',
              'appAttestKeyId': base64Encode(List<int>.filled(32, 7)),
            },
          ];
        }
        if (call.method == 'revokePushDelegation') {
          calls++;
          // This is the production Swift driver's durable pre-request mutation.
          grant = '';
          if (failGateway) {
            throw PlatformException(code: 'transient_gateway_failure');
          }
        }
        return null;
      });
      addTearDown(() {
        messenger.setMockMethodCallHandler(native, null);
        pushEndpointGrants.value = const [];
        apnsDeviceToken.value = null;
      });
      final overrides = [
        savedPrefsProvider.overrideWithValue(prefs),
        communityStorageProvider.overrideWithValue(storage),
        communitySnapshotWriterProvider.overrideWithValue((_) async {}),
        buzzPushLeaseRevocationOutboxProvider.overrideWith((ref) {
          final outbox = BuzzPushLeaseRevocationOutbox(
            storage: journal,
            publisher: publishBuzzPushLeaseRevocation,
            now: tester.binding.clock.now,
            jitter: () => 0,
          );
          ref.onDispose(outbox.dispose);
          return outbox;
        }),
        communityPushLeaseDeactivatorProvider.overrideWithValue((
          community, {
          generation,
        }) async {
          relayTombstones++;
          if (sessionDisconnected) throw StateError('relay offline');
        }),
      ];
      final initial = ProviderContainer(overrides: overrides);
      await initial.read(communityListProvider.future);
      await initial
          .read(communityListProvider.notifier)
          .setPushNotificationsEnabled(community.id, false);
      await initial.read(buzzPushLeaseRevocationOutboxProvider).trigger();
      expect(calls, 1);
      expect(grant, isEmpty);
      expect(
        (await journal.loadAll()).single.gatewayRevocation?['generation'],
        3,
      );
      expect(
        (await storage.loadAll())
            .single
            .pushSubscriptionState
            .pendingTombstoneGeneration,
        8,
      );
      expect(relayTombstones, 1);
      initial.dispose();

      failGateway = false; // gateway recovered; relay remains disconnected.
      final disabled = (await storage.loadAll()).single;
      final session = _ConnectedSession(
        initialStatus: SessionStatus.disconnected,
      );
      await tester.pumpWidget(
        ProviderScope(
          overrides: [
            ...overrides,
            relaySessionProvider.overrideWith(() => session),
            activeCommunityProvider.overrideWith((_) async => disabled),
            currentRelayPushDescriptorProvider.overrideWith((_) async => null),
            pushSubscriptionSyncProvider.overrideWith((_) async => 0),
          ],
          child: const BuzzPushBootstrap(child: SizedBox()),
        ),
      );
      for (var i = 0; i < 10; i++) {
        await tester.pump();
      }
      for (final seconds in [5, 30, 120]) {
        await tester.pump(Duration(seconds: seconds));
        await tester.pump();
      }
      expect(nativeReads, greaterThan(1)); // native inventory was reconciled.
      expect(calls, 2); // Durable retry ran without a relay connection.
      expect(await journal.loadAll(), isEmpty);
      expect(
        (await storage.loadAll())
            .single
            .pushSubscriptionState
            .pendingTombstoneGeneration,
        8,
      );
      sessionDisconnected = false;
      session.setStatus(SessionStatus.connected);
      for (var i = 0; i < 12; i++) {
        await tester.pump();
      }
      expect(calls, 2);
      expect(relayTombstones, 2);
      expect(
        (await storage.loadAll())
            .single
            .pushSubscriptionState
            .pendingTombstoneGeneration,
        isNull,
      );
      await tester.pumpWidget(const SizedBox.shrink());
      debugDefaultTargetPlatformOverride = null;
    },
  );

  testWidgets('first opt-in starts registration without switching community', (
    tester,
  ) async {
    var registrations = 0;
    Widget bootstrap(bool enabled) => BuzzPushRegistrationBootstrap(
      shouldRegister: enabled,
      attemptKey: 'same-community|same-relay',
      startRegistration: () async {
        registrations += 1;
      },
      child: const SizedBox(),
    );
    await tester.pumpWidget(bootstrap(false));
    expect(registrations, 0);
    await tester.pumpWidget(bootstrap(true));
    expect(registrations, 1);
    await tester.pumpWidget(bootstrap(true));
    expect(registrations, 1);
  });

  testWidgets('new community starts registration once capability arrives', (
    tester,
  ) async {
    final community = Community.create(
      name: 'Team',
      relayUrl: 'wss://relay.example',
    );
    var registrations = 0;
    Widget bootstrap(BuzzPushLeaseDescriptor? descriptor) =>
        BuzzPushRegistrationBootstrap(
          shouldRegister: buzzPushLifecycleEnabled(
            community: community,
            descriptor: descriptor,
          ),
          attemptKey: '${community.id}|${community.relayUrl}',
          startRegistration: () async => registrations += 1,
          child: const SizedBox(),
        );
    await tester.pumpWidget(bootstrap(null));
    expect(registrations, 0);
    final descriptor = _descriptor(keyId: 'relay-v1', pubkey: _hex('b'));
    await tester.pumpWidget(bootstrap(descriptor));
    expect(registrations, 1);
    await tester.pumpWidget(bootstrap(descriptor));
    expect(registrations, 1);
  });

  test(
    'superseded lease acceptance is a retryable publication failure',
    () async {
      await expectLater(
        publishBuzzPushLeaseRecoverably(
          reserveGeneration: () async => 1,
          publish: (_) async {},
          markAccepted: (_) async => false,
        ),
        throwsStateError,
      );
    },
  );

  test('failed bootstrap attempt becomes retryable after the delay', () async {
    final gate = BuzzPushAttemptGate(retryDelay: Duration.zero);
    addTearDown(gate.dispose);
    var retries = 0;

    expect(gate.tryBegin('attempt'), isTrue);
    gate.failed('attempt', retry: () => retries += 1);
    await Future<void>.delayed(Duration.zero);

    expect(retries, 1);
    expect(gate.tryBegin('attempt'), isTrue);
  });

  testWidgets('a failed attempt stays blocked until the retry delay', (
    tester,
  ) async {
    final gate = BuzzPushAttemptGate();
    addTearDown(gate.dispose);
    var retries = 0;
    expect(gate.tryBegin('attempt'), isTrue);
    gate.failed('attempt', retry: () => retries++);
    expect(gate.tryBegin('attempt'), isFalse);
    await tester.pump(const Duration(seconds: 4));
    expect(gate.tryBegin('attempt'), isFalse);
    expect(retries, 0);
    await tester.pump(const Duration(seconds: 1));
    expect(retries, 1);
    expect(gate.tryBegin('attempt'), isTrue);
  });

  test('a new attempt cancels an obsolete scheduled retry', () async {
    final gate = BuzzPushAttemptGate(retryDelay: Duration.zero);
    addTearDown(gate.dispose);
    var retries = 0;

    expect(gate.tryBegin('old'), isTrue);
    gate.failed('old', retry: () => retries += 1);
    expect(gate.tryBegin('new'), isTrue);
    await Future<void>.delayed(Duration.zero);

    expect(retries, 0);
    expect(gate.tryBegin('new'), isFalse);
  });

  test('successful bootstrap becomes retryable at renewal time', () async {
    final gate = BuzzPushAttemptGate(retryDelay: Duration.zero);
    addTearDown(gate.dispose);
    var retries = 0;

    expect(gate.tryBegin('attempt'), isTrue);
    gate.retryAfter('attempt', delay: Duration.zero, retry: () => retries += 1);
    await Future<void>.delayed(Duration.zero);

    expect(retries, 1);
    expect(gate.tryBegin('attempt'), isTrue);
  });

  test('completed bootstrap attempt can run again for later work', () {
    final gate = BuzzPushAttemptGate();
    addTearDown(gate.dispose);

    expect(gate.tryBegin('attempt'), isTrue);
    gate.complete('attempt');
    expect(gate.tryBegin('attempt'), isTrue);
  });

  test('publication attempt changes when the relay executor rotates', () {
    final subscription = BuzzPushSubscription(
      filter: BuzzPushFilter(kinds: const [9], pTags: [_hex('a')]),
      notificationClass: 'default',
    );
    final original = buzzPushPublicationAttemptKey(
      communityId: 'community',
      relayBaseUrl: 'https://relay.example',
      token: 'token',
      descriptor: _descriptor(keyId: 'relay-v1', pubkey: _hex('b')),
      subscriptions: [subscription],
    );

    expect(
      buzzPushPublicationAttemptKey(
        communityId: 'community',
        relayBaseUrl: 'https://relay.example',
        token: 'token',
        descriptor: _descriptor(keyId: 'relay-v2', pubkey: _hex('b')),
        subscriptions: [subscription],
      ),
      isNot(original),
    );
    expect(
      buzzPushPublicationAttemptKey(
        communityId: 'community',
        relayBaseUrl: 'https://relay.example',
        token: 'token',
        descriptor: _descriptor(keyId: 'relay-v1', pubkey: _hex('c')),
        subscriptions: [subscription],
      ),
      isNot(original),
    );
  });

  test('relay grammar upgrade changes publication attempt identity', () {
    String key(String grammar) => buzzPushPublicationAttemptKey(
      communityId: 'community',
      relayBaseUrl: 'https://relay.example',
      token: 'token',
      descriptor: _descriptor(
        keyId: 'relay-v1',
        pubkey: _hex('b'),
        hGrammar: grammar,
      ),
      subscriptions: const [],
    );
    expect(key('uuid-lowercase'), isNot(key('uuid-v4-lowercase')));
  });

  test('default push requires relay capability and respects opt-out', () {
    final enabled = Community.create(
      name: 'Team',
      relayUrl: 'wss://relay.example',
    );
    final disabled = enabled.copyWith(pushNotificationsEnabled: false);

    expect(
      buzzPushLifecycleEnabled(
        community: disabled,
        descriptor: _descriptor(keyId: 'relay-v1', pubkey: _hex('b')),
      ),
      isFalse,
    );
    expect(
      buzzPushLifecycleEnabled(
        community: enabled,
        descriptor: _descriptor(keyId: 'relay-v1', pubkey: _hex('b')),
      ),
      isTrue,
    );
    expect(
      buzzPushLifecycleEnabled(community: enabled, descriptor: null),
      isFalse,
    );
  });

  test('pending opt-out tombstone keeps active push lifecycle disabled', () {
    final subscription = BuzzPushSubscription(
      filter: BuzzPushFilter(kinds: const [9], pTags: [_hex('a')]),
      notificationClass: 'default',
    );
    final community =
        Community.create(
          name: 'Team',
          relayUrl: 'wss://relay.example',
        ).copyWith(
          pushNotificationsEnabled: false,
          pushSubscriptionState:
              BuzzPushLeaseSubscriptionState.desired(desired: [subscription])
                  .withAccepted(subscriptions: [subscription], generation: 3)
                  .withPendingTombstone(4),
        );

    expect(
      buzzPushLifecycleEnabled(
        community: community,
        descriptor: _descriptor(keyId: 'relay-v1', pubkey: _hex('b')),
      ),
      isFalse,
    );
  });

  test(
    'relay commit followed by local failure retries at a newer generation',
    () async {
      var durableCursor = 0;
      var relayGeneration = 0;
      var acceptedGeneration = 0;
      var failLocalSave = true;

      Future<int> reserve() async => ++durableCursor;
      Future<void> publish(int generation) async {
        expect(generation, greaterThan(relayGeneration));
        relayGeneration = generation;
      }

      Future<bool> markAccepted(int generation) async {
        if (failLocalSave) {
          failLocalSave = false;
          throw StateError('injected local persistence failure');
        }
        acceptedGeneration = generation;
        return true;
      }

      await expectLater(
        publishBuzzPushLeaseRecoverably(
          reserveGeneration: reserve,
          publish: publish,
          markAccepted: markAccepted,
        ),
        throwsStateError,
      );
      expect(relayGeneration, 1);
      expect(acceptedGeneration, 0);

      await publishBuzzPushLeaseRecoverably(
        reserveGeneration: reserve,
        publish: publish,
        markAccepted: markAccepted,
      );
      expect(relayGeneration, 2);
      expect(acceptedGeneration, 2);
    },
  );
}

BuzzPushLeaseDescriptor _descriptor({
  required String keyId,
  required String pubkey,
  String hGrammar = 'uuid-v4-lowercase',
}) => BuzzPushLeaseDescriptor(
  origin: 'wss://relay.example',
  hGrammar: hGrammar,
  executorKeyId: keyId,
  executorPubkey: pubkey,
  transport: 'apns',
  maxLeaseTtlSeconds: 3600,
  maxContentLength: 4096,
  maxPlaintextLength: 4096,
  maxEndpointLength: 2048,
  maxStringLength: 512,
);

String _hex(String character) => List.filled(64, character).join();

class _ConnectedSession extends RelaySessionNotifier {
  _ConnectedSession({required this.initialStatus});
  final SessionStatus initialStatus;
  @override
  SessionState build() => SessionState(status: initialStatus);
  void setStatus(SessionStatus value) => state = SessionState(status: value);
}
