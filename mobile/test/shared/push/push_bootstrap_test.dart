import 'dart:async' show Completer;
import 'dart:io';
import 'push_relay_fixture.dart';
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
  late Map<String, BuzzPushCommunityLeaseHandoff> handoffs;
  setUp(() => handoffs = {});
  testWidgets(
    'inactive relay failures preserve healthy handoffs and recover ambiguous acceptance',
    (tester) async {
      await HttpOverrides.runWithHttpOverrides(() async {
        final user = nostr.Keys('1' * 64);
        const oldHandle = '22222222-2222-4222-8222-222222222222';
        const newHandle = '44444444-4444-4444-8444-444444444444';
        const native = MethodChannel('buzz/push');
        final messenger = tester.binding.defaultBinaryMessenger;
        debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
        final healthy = await tester.runAsync(PushRelayFixture.start);
        final bad = await tester.runAsync(
          () => PushRelayFixture.start(failMetadata: true),
        );
        final good = healthy!;
        final failing = bad!;
        final storage = _FailAcceptedCommunityStorage(
          secure: FakeSecureStorage(),
        );
        final subscription = BuzzPushSubscription(
          filter: BuzzPushFilter(kinds: [9], pTags: [user.public]),
          notificationClass: 'default',
        );
        for (final r in [good, failing]) {
          await storage.save(
            Community.create(
              name: r.httpUrl,
              relayUrl: r.httpUrl,
              nsec: user.nsec,
              pubkey: user.public,
            ).copyWith(
              pushSubscriptionState: BuzzPushLeaseSubscriptionState.desired(
                desired: [subscription],
              ),
            ),
          );
        }
        final active = Community.create(
          name: 'active-disabled',
          relayUrl: 'https://active.invalid',
          nsec: user.nsec,
          pubkey: user.public,
        ).copyWith(pushNotificationsEnabled: false);
        await storage.save(active);
        Map<String, dynamic> grant(
          String origin,
          String handle,
          int generation,
        ) => {
          'relayOrigin': origin,
          'relayPubkey': relayKeys.public,
          'installationId': '0' * 32,
          'endpointGrant': 'valid-reusable-fixture-grant',
          'endpointHash': 'b' * 64,
          'endpointEpoch': 1,
          'generation': generation,
          'expiresAt': DateTime.now().millisecondsSinceEpoch ~/ 1000 + 86400,
          'gatewayInstallationHandle': handle,
          'appAttestKeyId': 'fixture-key',
        };
        final selection = Completer<Community?>();
        final inventory = [
          grant(good.wsUrl, oldHandle, 1),
          grant(failing.wsUrl, newHandle, 1),
        ];
        var nativeEnrollCalls = 0;
        messenger.setMockMethodCallHandler(native, (call) async {
          if (call.method == 'endpointGrants') return inventory;
          if (call.method == 'enrollPush') {
            nativeEnrollCalls++;
            final origin = (call.arguments as Map)['relayUrl'];
            return inventory.singleWhere((g) => g['relayOrigin'] == origin);
          }
          return null;
        });
        SharedPreferences.setMockInitialValues({});
        final prefs = await SharedPreferences.getInstance();
        apnsDeviceToken.value = 'fixture-token';
        pushEndpointGrants.value = [
          for (final g in inventory) BuzzPushEndpointGrant.fromMap(g),
        ];
        await tester.pumpWidget(
          ProviderScope(
            overrides: [
              savedPrefsProvider.overrideWithValue(prefs),
              communityStorageProvider.overrideWithValue(storage),
              communitySnapshotWriterProvider.overrideWithValue((_) async {}),
              buzzPushLeaseRevocationStorageProvider.overrideWithValue(
                BuzzPushLeaseRevocationStorage(secure: FakeSecureStorage()),
              ),
              activeCommunityProvider.overrideWith((_) => selection.future),
              relaySessionProvider.overrideWith(_Disconnected.new),
              pushSubscriptionSyncProvider.overrideWith((_) async => 0),
              currentRelayPushDescriptorProvider.overrideWith(
                (_) async => null,
              ),
            ],
            child: const BuzzPushBootstrap(child: SizedBox()),
          ),
        );
        Future<void> waitForBatch(int failed, int accepted) async {
          for (
            var i = 0;
            i < 100 &&
                (good.leases.length < accepted || failing.nip11 < failed);
            i++
          ) {
            await tester.runAsync(
              () => Future<void>.delayed(const Duration(milliseconds: 10)),
            );
            await tester.pump();
          }
          expect(good.leases, hasLength(accepted));
          expect(failing.nip11, greaterThanOrEqualTo(failed));
        }

        // No inactive publication may steal ownership while selection is unresolved.
        for (var i = 0; i < 10; i++) {
          await tester.pump();
        }
        expect(good.leases, isEmpty);
        selection.complete(active);
        await tester.pump();
        await waitForBatch(1, 1);
        await tester.pump(const Duration(seconds: 5));
        await waitForBatch(2, 1);
        await tester.pump(const Duration(seconds: 5));
        await waitForBatch(3, 1);
        expect(good.nip11, 1);
        expect(nativeEnrollCalls, 1);
        var saved = (await storage.loadAll()).firstWhere(
          (c) => c.relayUrl == good.httpUrl,
        );
        expect(saved.pushSubscriptionState.acceptedGeneration, 1);
        final container = ProviderScope.containerOf(
          tester.element(find.byType(BuzzPushBootstrap)),
        );
        await container
            .read(communityListProvider.notifier)
            .updateDesiredPushSubscriptions(saved.id, [
              BuzzPushSubscription(
                filter: BuzzPushFilter(kinds: [40002], pTags: [user.public]),
                notificationClass: 'default',
              ),
            ]);
        await tester.pump();
        await waitForBatch(4, 2);
        // Relay accepts the next handoff, but local acceptance persistence fails.
        // The same valid native grant and subscriptions must still be retried.
        storage.failNextAcceptedId = saved.id;
        apnsDeviceToken.value = 'rotated-token';
        await tester.pump();
        await waitForBatch(5, 3);
        for (var i = 0; i < 10; i++) {
          await tester.pump();
        }
        saved = (await storage.loadAll()).firstWhere((c) => c.id == saved.id);
        expect(saved.pushSubscriptionState.generationCursor, 3);
        expect(saved.pushSubscriptionState.acceptedGeneration, 2);
        await tester.pump(const Duration(seconds: 5));
        await waitForBatch(6, 4);
        for (var i = 0; i < 10; i++) {
          await tester.pump();
        }
        saved = (await storage.loadAll()).firstWhere((c) => c.id == saved.id);
        expect(saved.pushSubscriptionState.acceptedGeneration, 4);
        expect(good.leases.map((lease) => lease['generation']), [1, 2, 3, 4]);
        await tester.pumpWidget(const SizedBox.shrink());
        debugDefaultTargetPlatformOverride = null;
        await tester.runAsync(good.close);
        await tester.runAsync(failing.close);
        messenger.setMockMethodCallHandler(native, null);
        apnsDeviceToken.value = null;
        pushEndpointGrants.value = [];
      }, DirectPushHttp());
    },
  );
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

  test(
    'rotation renews both enabled origins and retries failed durable handoffs',
    () async {
      final communities = [
        _acceptedCommunity('one'),
        _acceptedCommunity('two'),
      ];
      final grants = [_grant('one', ''), _grant('two', '')];
      final calls = <String>[];
      var fail = true;
      Future<BuzzPushEndpointGrant> publish(Community community) async {
        calls.add(community.name);
        // Native issuance committed before the relay handoff failed. A retry
        // must include this now-nonempty grant as well as the other origin.
        final index = community.name == 'one' ? 0 : 1;
        grants[index] = _grant(community.name, 'g2_fresh');
        if (fail && index == 0) throw StateError('relay handoff unavailable');
        return grants[index];
      }

      await expectLater(
        renewBuzzPushCommunityLeases(
          token: 'token',
          completed: handoffs,
          now: () => DateTime.fromMillisecondsSinceEpoch(0),
          readCommunities: () => communities,
          grants: grants,
          publish: publish,
        ),
        throwsStateError,
      );
      expect(calls, ['one', 'two']);
      fail = false;
      calls.clear();
      expect(
        await renewBuzzPushCommunityLeases(
          token: 'token',
          completed: handoffs,
          now: () => DateTime.fromMillisecondsSinceEpoch(0),
          readCommunities: () => communities,
          grants: grants,
          publish: publish,
        ),
        3600,
      );
      expect(calls, ['one']);
    },
  );

  test(
    'confirmed handoffs renew only when due or their own inputs change',
    () async {
      var communities = [_acceptedCommunity('one'), _acceptedCommunity('two')];
      var grants = [_grant('one', 'saved'), _grant('two', 'saved')];
      var token = 'token';
      var seconds = 0;
      final calls = <String>[];
      Future<void> renew() async {
        calls.clear();
        await renewBuzzPushCommunityLeases(
          token: token,
          completed: handoffs,
          now: () => DateTime.fromMillisecondsSinceEpoch(seconds * 1000),
          readCommunities: () => communities,
          grants: grants,
          publish: (community) async {
            calls.add(community.name);
            return grants.singleWhere(
              (grant) => grant.relayOrigin == community.relayUrl,
            );
          },
        );
      }

      await renew();
      expect(calls, ['one', 'two']);
      for (final time in [5, 10, 3299]) {
        seconds = time;
        await renew();
        expect(calls, isEmpty);
      }
      seconds = 3300;
      await renew();
      expect(calls, ['one', 'two']);
      seconds = 0;
      token = 'rotated-token';
      await renew();
      expect(calls, ['one', 'two']);
      communities[0] = communities[0].copyWith(
        pushSubscriptionState: communities[0].pushSubscriptionState.withDesired(
          [
            BuzzPushSubscription(
              filter: BuzzPushFilter(kinds: [40002], pTags: [_hex('a')]),
              notificationClass: 'default',
            ),
          ],
        ),
      );
      await renew();
      expect(calls, ['one']);
      grants[0] = _grant('one', 'changed-authority');
      await renew();
      expect(calls, ['one']);
      communities[0] = communities[0].copyWith(nsec: 'changed-owner');
      await renew();
      expect(calls, ['one']);
      final removed = communities.removeAt(0);
      await renew();
      expect(handoffs.keys, [communities.single.id]);
      communities.insert(0, removed);
      await renew();
      expect(calls, ['one']);
      communities[0] = communities[0].copyWith(pushNotificationsEnabled: false);
      await renew();
      communities[0] = communities[0].copyWith(pushNotificationsEnabled: true);
      await renew();
      expect(calls, ['one']);
    },
  );

  test(
    'rotation rechecks opt-out and removal before each origin handoff',
    () async {
      var communities = [_acceptedCommunity('one'), _acceptedCommunity('two')];
      final calls = <String>[];
      await renewBuzzPushCommunityLeases(
        token: 'token',
        completed: handoffs,
        now: () => DateTime.fromMillisecondsSinceEpoch(0),
        readCommunities: () => communities,
        grants: [_grant('one', ''), _grant('two', '')],
        publish: (community) async {
          calls.add(community.name);
          communities = [
            communities.first,
            communities.last.copyWith(pushNotificationsEnabled: false),
          ];
          return _grant(community.name, 'g2_fresh');
        },
      );
      expect(calls, ['one']);
      handoffs.clear();
      await renewBuzzPushCommunityLeases(
        token: 'token',
        completed: handoffs,
        now: () => DateTime.fromMillisecondsSinceEpoch(0),
        readCommunities: () => communities,
        grants: [_grant('one', '')],
        publish: (community) async {
          communities[0] = community.copyWith(nsec: 'new owner during handoff');
          return _grant('one', 'g2_fresh');
        },
      );
      expect(
        handoffs,
        isEmpty,
        reason: "a stale owner cannot complete the new owner's handoff",
      );
      communities = [];
      await renewBuzzPushCommunityLeases(
        token: 'token',
        completed: handoffs,
        now: () => DateTime.fromMillisecondsSinceEpoch(0),
        readCommunities: () => communities,
        grants: [_grant('one', '')],
        publish: (_) async => throw StateError('removed community published'),
      );
    },
  );

  test(
    'rotation restores only origins with native authority and desired subscriptions',
    () async {
      final enrolled = _acceptedCommunity('one');
      final newCommunity = _acceptedCommunity('two').copyWith(
        pushSubscriptionState: const BuzzPushLeaseSubscriptionState.desired(),
      );
      final calls = <String>[];
      await renewBuzzPushCommunityLeases(
        token: 'token',
        completed: handoffs,
        now: () => DateTime.fromMillisecondsSinceEpoch(0),
        readCommunities: () => [
          enrolled,
          newCommunity,
          _acceptedCommunity('three'),
        ],
        grants: [_grant('one', ''), _grant('two', '')],
        publish: (community) async {
          calls.add(community.name);
          return _grant(community.name, 'g2_fresh');
        },
      );
      expect(calls, ['one']);
    },
  );

  test(
    'inactive first handoff retries its durable reserved generation',
    () async {
      final original = _acceptedCommunity('one');
      final pending = original.copyWith(
        pushSubscriptionState: BuzzPushLeaseSubscriptionState.desired(
          desired: original.pushSubscriptionState.desired,
        ).withReservedGeneration(1),
      );
      var attempts = 0;
      Future<BuzzPushEndpointGrant> publish(Community community) async {
        attempts++;
        if (attempts == 1) throw StateError('initial relay handoff lost');
        return _grant('one', 'g2_saved');
      }

      Future<int?> retry() => renewBuzzPushCommunityLeases(
        token: 'token',
        completed: handoffs,
        now: () => DateTime.fromMillisecondsSinceEpoch(0),
        readCommunities: () => [pending],
        grants: [_grant('one', 'g2_saved')],
        publish: publish,
      );
      await expectLater(retry(), throwsStateError);
      expect(await retry(), 3600);
      expect(attempts, 2);
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

Community _acceptedCommunity(String name) {
  final subscriptions = [
    BuzzPushSubscription(
      filter: BuzzPushFilter(kinds: const [9], pTags: [_hex('a')]),
      notificationClass: 'default',
    ),
  ];
  return Community.create(
    name: name,
    relayUrl: 'wss://$name.example',
    nsec: 'test-key',
  ).copyWith(
    pushNotificationsEnabled: true,
    pushSubscriptionState: BuzzPushLeaseSubscriptionState.desired(
      desired: subscriptions,
    ).withAccepted(subscriptions: subscriptions, generation: 1),
  );
}

BuzzPushEndpointGrant _grant(String origin, String grant) =>
    BuzzPushEndpointGrant(
      relayOrigin: 'wss://$origin.example',
      relayPubkey: _hex('b'),
      installationId: '0' * 32,
      endpointGrant: grant,
      endpointHash: _hex('c'),
      endpointEpoch: 2,
      generation: 2,
      expiresAt: 3600,
    );

class _ConnectedSession extends RelaySessionNotifier {
  _ConnectedSession({required this.initialStatus});
  final SessionStatus initialStatus;
  @override
  SessionState build() => SessionState(status: initialStatus);
  void setStatus(SessionStatus value) => state = SessionState(status: value);
}

class _Disconnected extends RelaySessionNotifier {
  @override
  SessionState build() =>
      const SessionState(status: SessionStatus.disconnected);
}

class _FailAcceptedCommunityStorage extends CommunityStorage {
  _FailAcceptedCommunityStorage({required super.secure});
  String? failNextAcceptedId;
  @override
  Future<void> save(Community community) async {
    final state = community.pushSubscriptionState;
    if (community.id == failNextAcceptedId &&
        state.generationCursor == state.acceptedGeneration) {
      failNextAcceptedId = null;
      throw StateError('injected accepted-generation save failure');
    }
    await super.save(community);
  }
}
