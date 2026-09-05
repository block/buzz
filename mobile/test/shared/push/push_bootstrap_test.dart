import 'package:buzz/shared/push/dev_push_lease.dart';
import 'package:buzz/shared/community/community.dart';
import 'package:buzz/shared/push/push_bootstrap.dart';
import 'package:buzz/shared/push/push_subscription.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  test('gateway cleanup retries exponentially and then stops', () {
    expect(
      [
        for (var failure = 1; failure <= 7; failure++)
          buzzPushGatewayInitializationRetryDelay(failure),
      ],
      const [
        Duration(seconds: 5),
        Duration(seconds: 10),
        Duration(seconds: 20),
        Duration(seconds: 40),
        Duration(seconds: 80),
        Duration(seconds: 160),
        null,
      ],
    );
    expect(
      () => buzzPushGatewayInitializationRetryDelay(0),
      throwsArgumentError,
    );
  });

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

  test('a new attempt cancels an obsolete scheduled retry', () async {
    final gate = BuzzPushAttemptGate(retryDelay: Duration.zero);
    addTearDown(gate.dispose);
    var retries = 0;

    expect(gate.tryBegin('old'), isTrue);
    gate.failed('old', retry: () => retries += 1);
    expect(gate.tryBegin('new'), isTrue);
    await Future<void>.delayed(Duration.zero);

    expect(retries, 0);
    expect(gate.isCurrent('old'), isFalse);
    expect(gate.isCurrent('new'), isTrue);
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

  test('relay capability alone does not activate push without opt-in', () {
    final disabled = Community.create(
      name: 'Team',
      relayUrl: 'wss://relay.example',
    );
    final enabled = disabled.copyWith(pushNotificationsEnabled: true);

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

  test('gateway migration includes inactive enabled communities', () {
    final active = Community.create(
      name: 'Active',
      relayUrl: 'https://active.example/path',
    ).copyWith(pushNotificationsEnabled: true);
    final inactive = Community.create(
      name: 'Inactive',
      relayUrl: 'wss://inactive.example',
    ).copyWith(pushNotificationsEnabled: true);
    final disabled = Community.create(
      name: 'Disabled',
      relayUrl: 'wss://disabled.example',
    );

    expect(
      buzzPushCommunitiesRequiringGatewayMigration(
        communities: [active, inactive, disabled],
        retiredRelayOrigins: const {
          'wss://active.example',
          'wss://inactive.example',
          'wss://disabled.example',
        },
        targetGatewayOrigin: 'https://push.example',
      ).map((community) => community.name),
      ['Active', 'Inactive'],
    );
  });

  testWidgets(
    'inactive migration work starts APNs registration through production boundary',
    (tester) async {
      final inactive = Community.create(
        name: 'Inactive',
        relayUrl: 'wss://inactive.example',
      ).copyWith(pushNotificationsEnabled: true);
      final migrationCommunities = buzzPushCommunitiesRequiringGatewayMigration(
        communities: [inactive],
        retiredRelayOrigins: const {'wss://inactive.example'},
        targetGatewayOrigin: 'https://push.example',
      );
      var registrations = 0;

      await tester.pumpWidget(
        MaterialApp(
          home: BuzzPushRegistrationBootstrap(
            shouldRegister: migrationCommunities.isNotEmpty,
            attemptKey: 'migration:${inactive.id}',
            startRegistration: () async => registrations += 1,
            child: const SizedBox(),
          ),
        ),
      );
      await tester.pump();

      expect(registrations, 1);
    },
  );

  test('gateway migration skips a durably checkpointed replacement', () {
    final community =
        Community.create(
          name: 'Migrated',
          relayUrl: 'wss://relay.example',
        ).copyWith(
          pushNotificationsEnabled: true,
          pushSubscriptionState: BuzzPushLeaseSubscriptionState.accepted(
            desired: const [],
            acceptedSubscriptions: const [],
            acceptedGeneration: 2,
            acceptedGatewayOrigin: 'https://push.example',
          ),
        );

    expect(
      buzzPushCommunitiesRequiringGatewayMigration(
        communities: [community],
        retiredRelayOrigins: const {'wss://relay.example'},
        targetGatewayOrigin: 'https://push.example',
      ),
      isEmpty,
    );
  });

  test('same-gateway rotation forces a durably checkpointed replacement', () {
    final community =
        Community.create(
          name: 'Rotated',
          relayUrl: 'wss://relay.example',
        ).copyWith(
          pushNotificationsEnabled: true,
          pushSubscriptionState: BuzzPushLeaseSubscriptionState.accepted(
            desired: const [],
            acceptedSubscriptions: const [],
            acceptedGeneration: 2,
            acceptedGatewayOrigin: 'https://push.example',
          ),
        );

    expect(
      buzzPushCommunitiesRequiringGatewayMigration(
        communities: [community],
        retiredRelayOrigins: const {'wss://relay.example'},
        replacementRelayOrigins: const {'wss://relay.example'},
        targetGatewayOrigin: 'https://push.example',
      ),
      [community],
    );
  });

  test('gateway migration rejects a stale APNs token before checkpoint', () {
    expect(
      buzzPushGatewayMigrationAttemptIsCurrent(
        attemptIsCurrent: true,
        token: 'old-token',
        liveToken: 'new-token',
        retiredRelayOrigins: const {'wss://relay.example'},
        liveRetiredRelayOrigins: const {'wss://relay.example'},
        replacementRelayOrigins: const {'wss://relay.example'},
        liveReplacementRelayOrigins: const {'wss://relay.example'},
        replacementGeneration: 7,
        liveReplacementGeneration: 7,
      ),
      isFalse,
    );
  });

  test(
    'gateway migration rejects a stale APNs token before acceptance',
    () async {
      var accepted = false;

      expect(
        await markBuzzPushGatewayMigrationAcceptedIfCurrent(
          attemptIsCurrent: () => false,
          markAccepted: () async {
            accepted = true;
            return true;
          },
        ),
        isFalse,
      );
      expect(accepted, isFalse);
    },
  );

  test('stale migration cannot start an authority mutation', () async {
    var mutated = false;

    await expectLater(
      runBuzzPushGatewayMigrationMutationIfCurrent<void>(
        attemptIsCurrent: () => false,
        mutate: () async {
          mutated = true;
        },
      ),
      throwsStateError,
    );
    expect(mutated, isFalse);
  });

  test('queued origins sharing delegation authority migrate atomically', () {
    final first = Community.create(
      name: 'First',
      relayUrl: 'wss://first.example',
    );
    final second = Community.create(
      name: 'Second',
      relayUrl: 'wss://second.example',
    );
    final independent = Community.create(
      name: 'Independent',
      relayUrl: 'wss://independent.example',
    );

    final groups = buzzPushGroupGatewayMigrationsByDelegationAuthority([
      (
        community: first,
        relayOrigin: 'wss://first.example',
        descriptor: _descriptor(keyId: 'first', pubkey: _hex('a')),
      ),
      (
        community: second,
        relayOrigin: 'wss://second.example',
        descriptor: _descriptor(keyId: 'second', pubkey: _hex('a')),
      ),
      (
        community: independent,
        relayOrigin: 'wss://independent.example',
        descriptor: _descriptor(keyId: 'third', pubkey: _hex('b')),
      ),
    ]);

    expect(groups[_hex('a')]!.map((target) => target.relayOrigin), [
      'wss://first.example',
      'wss://second.example',
    ]);
    expect(groups[_hex('b')]!.map((target) => target.relayOrigin), [
      'wss://independent.example',
    ]);

    expect(
      buzzPushGatewayMigrationGroupOriginsToQueue(
        targets: groups[_hex('a')]!,
        replacementRelayOrigins: const {'wss://first.example'},
      ),
      {'wss://first.example', 'wss://second.example'},
    );
    expect(
      buzzPushGatewayMigrationGroupOriginsToQueue(
        targets: groups[_hex('a')]!,
        replacementRelayOrigins: const {
          'wss://first.example',
          'wss://second.example',
        },
      ),
      isEmpty,
    );
  });

  test('migration retry budget resets for a new attempt generation', () {
    final budget = BuzzPushAttemptFailureBudget();

    expect(budget.recordFailure('generation-1'), 1);
    expect(budget.recordFailure('generation-1'), 2);
    expect(budget.recordFailure('generation-2'), 1);
    expect(budget.recordFailure('generation-2'), 2);
    budget.clear('generation-1');
    expect(budget.recordFailure('generation-2'), 3);
    budget.clear('generation-2');
    expect(budget.recordFailure('generation-2'), 1);
  });

  test('descriptor resolution preserves reachable migration work', () async {
    final reachable = Community.create(
      name: 'Reachable',
      relayUrl: 'wss://reachable.example',
    );
    final offline = Community.create(
      name: 'Offline',
      relayUrl: 'wss://offline.example',
    );

    final resolution = await resolveBuzzPushGatewayMigrationTargets(
      communities: [offline, reachable],
      fetchDescriptor: (relayUrl) async {
        if (relayUrl == offline.relayUrl) {
          throw StateError('offline');
        }
        return _descriptor(keyId: 'reachable', pubkey: _hex('a'));
      },
    );

    expect(resolution.targets.map((target) => target.community), [reachable]);
    expect(resolution.blockedOrigins, {'wss://offline.example'});
    expect(resolution.throwIfFailed, throwsStateError);
  });

  test(
    'migration processes later authority groups before propagating',
    () async {
      final processed = <String>[];

      await expectLater(
        processBuzzPushGatewayMigrationGroups(
          groups: const ['unavailable', 'reachable'],
          process: (group) async {
            processed.add(group);
            if (group == 'unavailable') throw StateError('rejected');
            return false;
          },
        ),
        throwsStateError,
      );

      expect(processed, ['unavailable', 'reachable']);
    },
  );

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

  test('superseded lease acceptance fails the publication attempt', () async {
    await expectLater(
      publishBuzzPushLeaseRecoverably(
        reserveGeneration: () async => 3,
        publish: (_) async {},
        markAccepted: (_) async => false,
      ),
      throwsStateError,
    );
  });

  test(
    'stale migration cannot reserve or publish a lease generation',
    () async {
      var reserved = false;
      var published = false;

      await expectLater(
        publishBuzzPushLeaseRecoverably(
          operationIsCurrent: () => false,
          reserveGeneration: () async {
            reserved = true;
            return 3;
          },
          publish: (_) async => published = true,
          markAccepted: (_) async => true,
        ),
        throwsStateError,
      );
      expect(reserved, isFalse);
      expect(published, isFalse);
    },
  );

  test('migration becoming stale during reservation cannot publish', () async {
    var current = true;
    var published = false;

    await expectLater(
      publishBuzzPushLeaseRecoverably(
        operationIsCurrent: () => current,
        reserveGeneration: () async {
          current = false;
          return 3;
        },
        publish: (_) async => published = true,
        markAccepted: (_) async => true,
      ),
      throwsStateError,
    );
    expect(published, isFalse);
  });
}

BuzzPushLeaseDescriptor _descriptor({
  required String keyId,
  required String pubkey,
}) => BuzzPushLeaseDescriptor(
  origin: 'wss://relay.example',
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
