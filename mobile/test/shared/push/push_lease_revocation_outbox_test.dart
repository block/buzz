import 'dart:async';
import 'dart:io';

import 'push_relay_fixture.dart';

import 'package:buzz/shared/community/community.dart';
import 'package:buzz/shared/community/community_provider.dart';
import 'package:buzz/shared/community/community_storage.dart';
import 'package:buzz/shared/push/push_bridge.dart';
import 'package:buzz/shared/push/push_lease_revocation_outbox.dart';
import 'package:buzz/shared/push/push_subscription.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;

import '../community/community_storage_test.dart';

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  late FakeSecureStorage secure;
  late BuzzPushLeaseRevocationStorage storage;
  late _Clock clock;
  late _WakeScheduler scheduler;
  late nostr.Keys keys;

  setUp(() {
    secure = FakeSecureStorage();
    storage = BuzzPushLeaseRevocationStorage(secure: secure);
    clock = _Clock(1_000_000);
    scheduler = _WakeScheduler();
    keys = nostr.Keys.generate();
  });

  group('production cleanup publisher', () {
    const channel = MethodChannel('buzz/push');
    final messenger =
        TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger;

    setUp(() => debugDefaultTargetPlatformOverride = TargetPlatform.iOS);
    tearDown(() {
      messenger.setMockMethodCallHandler(channel, null);
      debugDefaultTargetPlatformOverride = null;
      pushEndpointGrants.value = [];
      pushLeaseCleanupError.value = null;
    });

    for (final (siblingSigner, differentInstallation) in [
      (null, false),
      ('c' * 64, false),
      ('a' * 64, false),
      ('a' * 64, true),
    ]) {
      test(
        'only a shared installation/signer retains authority: $siblingSigner/$differentInstallation',
        () async {
          final sibling = Community.create(
            name: 'Unrelated or shared',
            relayUrl: 'wss://other.example',
          );
          final communities = CommunityStorage(secure: secure);
          await communities.save(sibling);
          final grant = _grant(
            expiresAt: DateTime.now().millisecondsSinceEpoch ~/ 1000 + 3600,
          );
          final grants = [
            grant,
            if (siblingSigner != null)
              BuzzPushEndpointGrant(
                relayOrigin: 'wss://other.example',
                relayPubkey: siblingSigner,
                installationId: grant.installationId,
                endpointGrant: 'opaque',
                endpointHash: grant.endpointHash,
                endpointEpoch: 1,
                generation: 1,
                expiresAt: grant.expiresAt,
                gatewayInstallationHandle: differentInstallation
                    ? '44444444-4444-4444-8444-444444444444'
                    : grant.gatewayInstallationHandle,
                appAttestKeyId: grant.appAttestKeyId,
              ),
          ];
          final revoked = <Object?>[];
          messenger.setMockMethodCallHandler(channel, (call) async {
            if (call.method == 'endpointGrants') {
              return [for (final value in grants) _nativeGrant(value)];
            }
            if (call.method == 'revokePushDelegation') {
              revoked.add(call.arguments);
              return null;
            }
            throw StateError('Unexpected native call: ${call.method}');
          });
          final container = ProviderContainer(
            overrides: [
              communityStorageProvider.overrideWithValue(communities),
              communitySnapshotWriterProvider.overrideWithValue((_) async {}),
              communityPushLeaseDeactivatorProvider.overrideWithValue(
                (_, {generation}) async {},
              ),
              buzzPushLeaseRevocationStorageProvider.overrideWithValue(storage),
            ],
          );
          addTearDown(container.dispose);
          await container.read(communityListProvider.future);
          final outbox = container.read(buzzPushLeaseRevocationOutboxProvider);
          // A was removed before B was added or its signer was discovered.
          await outbox.reconcileGatewayCleanup(
            [],
            readGrants: () async => [grant],
          );
          expect(await storage.loadAll(), hasLength(1));
          await outbox.trigger();
          expect(
            revoked,
            siblingSigner == grant.relayPubkey && !differentInstallation
                ? isEmpty
                : [grant.gatewayRevocationTarget],
          );
          expect(await storage.loadAll(), isEmpty);
          final shared =
              siblingSigner == grant.relayPubkey && !differentInstallation;
          await outbox.reconcileGatewayCleanup([
            sibling,
          ], readGrants: () async => grants);
          expect(await storage.loadAll(), hasLength(shared ? 0 : 1));
          await storage.replaceAll([]);
          final removed =
              Community.create(
                name: 'Removed',
                relayUrl: grant.relayOrigin,
                nsec: keys.nsec,
              ).copyWith(
                pushSubscriptionState:
                    const BuzzPushLeaseSubscriptionState.desired()
                        .withReservedGeneration(4),
              );
          await outbox.enqueueCommunity(
            removed,
            readGrants: () async => grants,
            remainingCommunities: [sibling],
          );
          expect(
            (await storage.loadAll())
                .where((record) => record.gatewayOnly)
                .map((record) => record.gatewayRevocation),
            shared ? isEmpty : [grant.gatewayRevocationTarget],
          );
          await storage.replaceAll([]);
          await communities.save(removed);
          container.invalidate(communityListProvider);
          await container.read(communityListProvider.future);
          revoked.clear();
          await container
              .read(communityListProvider.notifier)
              .setPushNotificationsEnabled(removed.id, false);
          await outbox.trigger();
          expect(revoked, shared ? isEmpty : [grant.gatewayRevocationTarget]);
        },
      );
    }

    test(
      'opt-out reaches relay while gateway cleanup remains durable',
      () async {
        await HttpOverrides.runWithHttpOverrides(() async {
          final relay = await PushRelayFixture.start();
          addTearDown(relay.close);
          final communities = CommunityStorage(secure: secure);
          final subscriptions = [
            BuzzPushSubscription(
              filter: BuzzPushFilter(kinds: [9], pTags: [keys.public]),
              notificationClass: 'default',
            ),
          ];
          final community =
              Community.create(
                name: 'Team',
                relayUrl: relay.httpUrl,
                nsec: keys.nsec,
                pubkey: keys.public,
              ).copyWith(
                pushSubscriptionState: BuzzPushLeaseSubscriptionState.desired(
                  desired: subscriptions,
                ).withAccepted(subscriptions: subscriptions, generation: 7),
              );
          await communities.save(community);
          final grant = _nativeGrant(
            _grant(
              expiresAt: DateTime.now().millisecondsSinceEpoch ~/ 1000 + 3600,
            ),
          )..['relayOrigin'] = relay.wsUrl;
          var calls = 0;
          messenger.setMockMethodCallHandler(channel, (call) async {
            if (call.method == 'endpointGrants') return [grant];
            if (call.method == 'revokePushDelegation') {
              calls++;
              grant['endpointGrant'] = '';
              throw PlatformException(code: 'app_attest_unavailable');
            }
            return null;
          });
          final container = ProviderContainer(
            overrides: [
              communityStorageProvider.overrideWithValue(communities),
              communitySnapshotWriterProvider.overrideWithValue((_) async {}),
              buzzPushLeaseRevocationStorageProvider.overrideWithValue(storage),
            ],
          );
          await container.read(communityListProvider.future);
          await container
              .read(communityListProvider.notifier)
              .setPushNotificationsEnabled(community.id, false);
          await container.read(buzzPushLeaseRevocationOutboxProvider).trigger();
          expect(calls, 1);
          expect(relay.leases.single['active'], isFalse);
          expect(relay.leases.single['generation'], 8);
          final disabled = (await communities.loadAll()).single;
          expect(disabled.pushNotificationsEnabled, isFalse);
          expect(
            disabled.pushSubscriptionState.pendingTombstoneGeneration,
            isNull,
          );
          final pending = (await storage.loadAll()).single;
          expect(pending.gatewayOnly, isTrue);
          expect(
            pending.gatewayRevocation?['installationHandle'],
            grant['gatewayInstallationHandle'],
          );
          container.dispose();
        }, DirectPushHttp());
      },
    );

    for (final gatewayFails in [true, false]) {
      test(
        'independent cleanup survives restart: gateway fails=$gatewayFails',
        () async {
          await HttpOverrides.runWithHttpOverrides(() async {
            final relay = await PushRelayFixture.start(
              failMetadata: !gatewayFails,
            );
            addTearDown(relay.close);
            var fail = true;
            var calls = 0;
            final grant = _grant(
              expiresAt: DateTime.now().millisecondsSinceEpoch ~/ 1000 + 3600,
            );
            messenger.setMockMethodCallHandler(channel, (call) async {
              if (call.method == 'revokePushDelegation') {
                calls++;
                expect(call.arguments, grant.gatewayRevocationTarget);
                if (fail && gatewayFails)
                  throw PlatformException(code: 'app_attest_unavailable');
              }
              return null;
            });
            clock.seconds = DateTime.now().millisecondsSinceEpoch ~/ 1000;
            BuzzPushLeaseRevocationOutbox makeOutbox() => _outbox(
              storage: storage,
              clock: clock,
              scheduler: scheduler,
              publisher: publishBuzzPushLeaseRevocation,
            );
            var outbox = makeOutbox();
            await outbox.enqueue(
              BuzzPushLeaseRevocationRecord(
                relayUrl: relay.httpUrl,
                relayOrigin: relay.wsUrl,
                memberPubkey: keys.public,
                nsec: keys.nsec,
                installationId: '0' * 32,
                generation: 8,
                expiresAt: grant.expiresAt,
                attemptCount: 0,
                nextAttemptAt: clock.seconds,
                gatewayRevocation: grant.gatewayRevocationTarget,
              ),
            );
            await outbox.trigger();
            outbox.dispose();
            outbox = makeOutbox();
            for (var retry = 0; retry < 2; retry++) {
              clock.seconds += 300;
              await outbox.trigger();
            }
            expect(calls, gatewayFails ? 3 : 1);
            expect(relay.leases.length, gatewayFails ? 1 : 0);
            final unfinished = (await storage.loadAll()).single;
            expect(unfinished.gatewayOnly, gatewayFails);
            if (gatewayFails) expect(relay.leases.single['active'], isFalse);
            fail = false;
            relay.failMetadata = false;
            clock.seconds += 300;
            await outbox.trigger();
            expect(await storage.loadAll(), isEmpty);
            expect(calls, gatewayFails ? 4 : 1);
            expect(relay.leases.single['active'], isFalse);
            outbox.dispose();
          }, DirectPushHttp());
        },
      );
    }

    test(
      'native revocation failure retains the exact cleanup journal',
      () async {
        final grant = _grant(
          expiresAt: DateTime.now().millisecondsSinceEpoch ~/ 1000 + 3600,
        );
        final communities = CommunityStorage(secure: secure);
        await communities.save(
          Community.create(
            name: 'New unrelated community',
            relayUrl: 'wss://other.example',
          ),
        );
        messenger.setMockMethodCallHandler(channel, (call) async {
          if (call.method == 'endpointGrants') return [_nativeGrant(grant)];
          if (call.method == 'revokePushDelegation') {
            throw PlatformException(code: 'enrollment_in_progress');
          }
          throw StateError('Unexpected native call: ${call.method}');
        });
        final container = ProviderContainer(
          overrides: [
            communityStorageProvider.overrideWithValue(communities),
            communitySnapshotWriterProvider.overrideWithValue((_) async {}),
            buzzPushLeaseRevocationStorageProvider.overrideWithValue(storage),
          ],
        );
        addTearDown(container.dispose);
        await container.read(communityListProvider.future);
        final outbox = container.read(buzzPushLeaseRevocationOutboxProvider);
        await outbox.reconcileGatewayCleanup(
          [],
          readGrants: () async => [grant],
        );
        await outbox.trigger();
        final pending = (await storage.loadAll()).single;
        expect(pending.gatewayRevocation, grant.gatewayRevocationTarget);
        expect(pending.attemptCount, 1);
        expect(
          pending.nextAttemptAt,
          greaterThan(DateTime.now().millisecondsSinceEpoch ~/ 1000),
        );
      },
    );
  });

  test('community removal journals one precise lease address', () async {
    final outbox = _outbox(
      storage: storage,
      clock: clock,
      scheduler: scheduler,
      publisher: (_) async {},
    );
    final community =
        Community.create(
          name: 'Test',
          relayUrl: 'wss://relay.example',
          nsec: keys.nsec,
        ).copyWith(
          pubkey: keys.public,
          pushNotificationsEnabled: true,
          pushSubscriptionState: const BuzzPushLeaseSubscriptionState.desired()
              .withReservedGeneration(4),
        );
    final grant = _grant(expiresAt: clock.seconds + 3600);

    await outbox.enqueueCommunity(community, readGrants: () async => [grant]);
    await outbox.enqueueCommunity(community, readGrants: () async => [grant]);

    final records = await storage.loadAll();
    expect(records, hasLength(2));
    final relay = records.singleWhere((record) => !record.gatewayOnly);
    expect(
      relay.leaseAddress,
      '${keys.public}|wss://relay.example|${community.pushLeaseInstallationId}',
    );
    expect(relay.generation, 6);
    expect(relay.expiresAt, grant.expiresAt);
    expect(relay.relayUrl, 'https://relay.example/');
  });

  test(
    'gateway target survives failed cleanup and is omitted for shared signer',
    () async {
      final community =
          Community.create(
            name: 'First',
            relayUrl: 'wss://relay.example',
            nsec: keys.nsec,
          ).copyWith(
            pubkey: keys.public,
            pushNotificationsEnabled: true,
            pushSubscriptionState:
                const BuzzPushLeaseSubscriptionState.desired()
                    .withReservedGeneration(4),
          );
      final grant = _grant(expiresAt: clock.seconds + 3600);
      final outbox = _outbox(
        storage: storage,
        clock: clock,
        scheduler: scheduler,
        publisher: (_) async => throw StateError('gateway response lost'),
      );
      await outbox.enqueueCommunity(community, readGrants: () async => [grant]);
      final before = (await storage.loadAll())
          .singleWhere((record) => record.gatewayOnly)
          .gatewayRevocation;
      expect(before?['generation'], 1);
      expect(
        before?['installationHandle'],
        '22222222-2222-4222-8222-222222222222',
      );
      await outbox.trigger();
      expect(
        (await storage.loadAll())
            .singleWhere((record) => record.gatewayOnly)
            .gatewayRevocation,
        before,
      );
      await storage.replaceAll([]);
      final sibling = Community.create(
        name: 'Second',
        relayUrl: 'wss://relay.example',
        nsec: keys.nsec,
      ).copyWith(pushNotificationsEnabled: true);
      await outbox.enqueueCommunity(
        community,
        readGrants: () async => [grant],
        remainingCommunities: [sibling],
      );
      expect((await storage.loadAll()).single.gatewayRevocation, isNull);
    },
  );

  test(
    'canceled first enrollment remains durable past its old expiry',
    () async {
      final community = Community.create(
        name: 'Canceled',
        relayUrl: 'wss://relay.example',
      ).copyWith(pushNotificationsEnabled: false);
      final pending = BuzzPushEndpointGrant(
        pending: true,
        relayOrigin: 'wss://relay.example',
        relayPubkey: 'a' * 64,
        installationId: '0' * 32,
        endpointGrant: '',
        endpointHash: 'b' * 64,
        endpointEpoch: 1,
        generation: 1,
        expiresAt: clock.seconds + 1,
      );
      var attempts = 0;
      final outbox = _outbox(
        storage: storage,
        clock: clock,
        scheduler: scheduler,
        publisher: (_) async {
          attempts += 1;
          throw StateError('native enrollment still in flight');
        },
      );
      await outbox.reconcileGatewayCleanup([
        community,
      ], readGrants: () async => [pending]);
      final target = (await storage.loadAll()).single;
      expect(target.gatewayOnly, isTrue);
      expect(target.gatewayRevocation?['relayOrigin'], pending.relayOrigin);
      expect(target.gatewayRevocation?['installationHandle'], isNull);
      clock.seconds += 2;
      await outbox.trigger();
      expect(attempts, 1);
      expect(await storage.loadAll(), hasLength(1));
      await storage.replaceAll([]);
      await outbox.reconcileGatewayCleanup([
        community.copyWith(pushNotificationsEnabled: true),
      ], readGrants: () async => [pending]);
      expect(
        await storage.loadAll(),
        isEmpty,
        reason: 'an enabled sibling retains shared authority',
      );
    },
  );

  test(
    'gateway cleanup retains separate authorities after relay key rotation',
    () async {
      final outbox = _outbox(
        storage: storage,
        clock: clock,
        scheduler: scheduler,
        publisher: (_) async => throw StateError('offline'),
      );
      final first = _grant(expiresAt: clock.seconds + 3600);
      await outbox.reconcileGatewayCleanup([], readGrants: () async => [first]);
      await outbox.trigger();
      expect((await storage.loadAll()).single.generation, 2);
      final second = _grant(
        expiresAt: clock.seconds + 3600,
        relayPubkey: 'c' * 64,
      );
      await outbox.reconcileGatewayCleanup(
        [],
        readGrants: () async => [second],
      );
      final records = await storage.loadAll();
      expect(records, hasLength(2));
      expect(
        records
            .map((record) => record.gatewayRevocation?['relayPubkey'])
            .toSet(),
        {'a' * 64, 'c' * 64},
      );
      expect(records.last.generation, 1);
    },
  );

  test(
    'gateway version does not become the JSON-safe local retry counter',
    () async {
      final outbox = _outbox(
        storage: storage,
        clock: clock,
        scheduler: scheduler,
        publisher: (_) async {},
      );
      const maximum = 9223372036854775807;
      await outbox.reconcileGatewayCleanup(
        [],
        readGrants: () async => [
          _grant(expiresAt: clock.seconds + 3600, generation: maximum),
        ],
      );
      final record = (await storage.loadAll()).single;
      expect(record.generation, 1);
      expect(record.gatewayRevocation?['generation'], maximum);
    },
  );

  test('legacy community revokes its grant-addressed lease', () async {
    final outbox = _outbox(
      storage: storage,
      clock: clock,
      scheduler: scheduler,
      publisher: (_) async {},
    );
    final community = Community(
      id: 'legacy',
      name: 'Legacy',
      relayUrl: 'wss://relay.example',
      pubkey: keys.public,
      nsec: keys.nsec,
      pushNotificationsEnabled: true,
      pushSubscriptionState: const BuzzPushLeaseSubscriptionState.desired()
          .withReservedGeneration(4),
      addedAt: DateTime.fromMillisecondsSinceEpoch(0),
    );
    final grant = _grant(expiresAt: clock.seconds + 3600);

    await outbox.enqueueCommunity(community, readGrants: () async => [grant]);

    expect(
      (await storage.loadAll())
          .singleWhere((record) => !record.gatewayOnly)
          .leaseAddress,
      '${keys.public}|wss://relay.example|${grant.installationId}',
    );
  });

  test('concurrent reconnect and resume triggers share one attempt', () async {
    final started = Completer<void>();
    final release = Completer<void>();
    var calls = 0;
    final outbox = _outbox(
      storage: storage,
      clock: clock,
      scheduler: scheduler,
      publisher: (_) async {
        calls += 1;
        if (!started.isCompleted) started.complete();
        await release.future;
      },
    );
    await outbox.enqueue(_record(keys: keys, clock: clock));

    final startup = outbox.start();
    await started.future;
    final reconnect = outbox.trigger();
    final resume = outbox.trigger();

    expect(calls, 1);
    expect(identical(reconnect, resume), isTrue);
    release.complete();
    await Future.wait([startup, reconnect, resume]);
    expect(calls, 1);
    expect(await storage.loadAll(), isEmpty);
  });

  test('failure reserves durable backoff and restart waits for it', () async {
    var failedCalls = 0;
    final first = _outbox(
      storage: storage,
      clock: clock,
      scheduler: scheduler,
      publisher: (_) async {
        failedCalls += 1;
        throw StateError('offline');
      },
    );
    await first.enqueue(_record(keys: keys, clock: clock));
    await first.trigger();

    final pending = (await storage.loadAll()).single;
    expect(failedCalls, 1);
    expect(pending.attemptCount, 1);
    expect(pending.generation, 8);
    expect(pending.nextAttemptAt, clock.seconds + 15);

    var restartedCalls = 0;
    final restarted = _outbox(
      storage: storage,
      clock: clock,
      scheduler: scheduler,
      publisher: (_) async => restartedCalls += 1,
    );
    await restarted.start();
    await restarted.trigger();
    await restarted.trigger();
    expect(restartedCalls, 0);

    clock.seconds = pending.nextAttemptAt;
    await restarted.trigger();
    expect(restartedCalls, 1);
    expect(await storage.loadAll(), isEmpty);
  });

  test('repeated failed restarts cannot create a retry storm', () async {
    await storage.replaceAll([_record(keys: keys, clock: clock)]);
    var calls = 0;

    for (var restart = 0; restart < 4; restart += 1) {
      final outbox = _outbox(
        storage: storage,
        clock: clock,
        scheduler: scheduler,
        publisher: (_) async {
          calls += 1;
          throw StateError('offline');
        },
      );
      await outbox.start();
      await Future.wait([outbox.trigger(), outbox.trigger(), outbox.trigger()]);
      outbox.dispose();
    }

    expect(calls, 1);
    final pending = (await storage.loadAll()).single;
    expect(pending.attemptCount, 1);
    expect(pending.nextAttemptAt, greaterThan(clock.seconds));
  });

  test('retry delay grows exponentially and remains bounded', () async {
    await storage.replaceAll([
      _record(
        keys: keys,
        clock: clock,
      ).copyWith(expiresAt: clock.seconds + 10_000_000),
    ]);
    final outbox = _outbox(
      storage: storage,
      clock: clock,
      scheduler: scheduler,
      publisher: (_) async => throw StateError('offline'),
    );
    final delays = <int>[];

    for (var attempt = 0; attempt < 14; attempt += 1) {
      await outbox.trigger();
      final pending = (await storage.loadAll()).single;
      delays.add(pending.nextAttemptAt - clock.seconds);
      clock.seconds = pending.nextAttemptAt;
    }

    expect(delays.take(4), [15, 30, 60, 120]);
    expect(delays.every((delay) => delay <= 6 * 60 * 60), isTrue);
    expect(delays.last, 3 * 60 * 60);
  });

  test('expired lease is erased without a relay attempt', () async {
    final expired = _record(
      keys: keys,
      clock: clock,
    ).copyWith(expiresAt: clock.seconds);
    await storage.replaceAll([expired]);
    var calls = 0;
    final outbox = _outbox(
      storage: storage,
      clock: clock,
      scheduler: scheduler,
      publisher: (_) async => calls += 1,
    );

    await outbox.start();

    expect(calls, 0);
    expect(await storage.loadAll(), isEmpty);
  });

  test('different records are globally serialized', () async {
    final firstStarted = Completer<void>();
    final releaseFirst = Completer<void>();
    var inFlight = 0;
    var maximumInFlight = 0;
    var calls = 0;
    final outbox = _outbox(
      storage: storage,
      clock: clock,
      scheduler: scheduler,
      publisher: (_) async {
        calls += 1;
        inFlight += 1;
        maximumInFlight = inFlight > maximumInFlight
            ? inFlight
            : maximumInFlight;
        if (calls == 1) {
          firstStarted.complete();
          await releaseFirst.future;
        }
        inFlight -= 1;
      },
    );
    await outbox.enqueue(_record(keys: keys, clock: clock));
    final otherKeys = nostr.Keys.generate();
    await outbox.enqueue(
      _record(keys: otherKeys, clock: clock, installationId: '1' * 32),
    );

    final draining = outbox.trigger();
    await firstStarted.future;
    expect(calls, 1);
    releaseFirst.complete();
    await draining;

    expect(calls, 2);
    expect(maximumInFlight, 1);
    expect(await storage.loadAll(), isEmpty);
  });
}

BuzzPushLeaseRevocationOutbox _outbox({
  required BuzzPushLeaseRevocationStorage storage,
  required _Clock clock,
  required _WakeScheduler scheduler,
  required BuzzPushLeaseRevocationPublisher publisher,
}) => BuzzPushLeaseRevocationOutbox(
  storage: storage,
  publisher: publisher,
  now: clock.call,
  jitter: () => 0,
  reportError: (_, _) {},
  scheduleWake: scheduler.schedule,
);

BuzzPushLeaseRevocationRecord _record({
  required nostr.Keys keys,
  required _Clock clock,
  String installationId = '00000000000000000000000000000000',
}) => BuzzPushLeaseRevocationRecord(
  relayUrl: 'https://relay.example',
  relayOrigin: 'wss://relay.example',
  memberPubkey: keys.public,
  nsec: keys.nsec,
  installationId: installationId,
  generation: 7,
  expiresAt: clock.seconds + 3600,
  attemptCount: 0,
  nextAttemptAt: clock.seconds,
);

BuzzPushEndpointGrant _grant({
  required int expiresAt,
  String? relayPubkey,
  int generation = 1,
}) => BuzzPushEndpointGrant(
  gatewayInstallationHandle: '22222222-2222-4222-8222-222222222222',
  appAttestKeyId: 'installed-app-attest-key',
  relayOrigin: 'wss://relay.example',
  relayPubkey: relayPubkey ?? 'a' * 64,
  installationId: '0' * 32,
  endpointGrant: 'opaque',
  endpointHash: 'b' * 64,
  endpointEpoch: 1,
  generation: generation,
  expiresAt: expiresAt,
);

class _Clock {
  _Clock(this.seconds);

  int seconds;

  DateTime call() => DateTime.fromMillisecondsSinceEpoch(seconds * 1000);
}

class _WakeScheduler {
  final List<_ScheduledWake> _wakes = [];

  void Function() schedule(Duration delay, void Function() callback) {
    final wake = _ScheduledWake();
    _wakes.add(wake);
    return () => wake.cancelled = true;
  }
}

class _ScheduledWake {
  bool cancelled = false;
}

Map<String, Object?> _nativeGrant(BuzzPushEndpointGrant grant) => {
  'relayOrigin': grant.relayOrigin,
  'relayPubkey': grant.relayPubkey,
  'installationId': grant.installationId,
  'endpointGrant': grant.endpointGrant,
  'endpointHash': grant.endpointHash,
  'endpointEpoch': grant.endpointEpoch,
  'generation': grant.generation,
  'expiresAt': grant.expiresAt,
  'gatewayInstallationHandle': grant.gatewayInstallationHandle,
  'appAttestKeyId': grant.appAttestKeyId,
};
