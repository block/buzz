import 'package:buzz/shared/push/dev_push_lease.dart';
import 'package:buzz/shared/community/community.dart';
import 'package:buzz/shared/push/push_bootstrap.dart';
import 'package:buzz/shared/push/push_bridge.dart';
import 'package:buzz/shared/push/push_subscription.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:flutter/widgets.dart';

void main() {
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
          readCommunities: () => communities,
          grants: grants,
          publish: publish,
        ),
        3600,
      );
      expect(calls, ['one', 'two']);
    },
  );

  test(
    'rotation rechecks opt-out and removal before each origin handoff',
    () async {
      var communities = [_acceptedCommunity('one'), _acceptedCommunity('two')];
      final calls = <String>[];
      await renewBuzzPushCommunityLeases(
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
      communities = [];
      await renewBuzzPushCommunityLeases(
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
