import 'dart:math';

import 'package:buzz/shared/push/dev_push_lease.dart';
import 'package:buzz/shared/community/community.dart';
import 'package:buzz/shared/push/push_bootstrap.dart';
import 'package:buzz/shared/push/push_subscription.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:flutter/widgets.dart';

void main() {
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
    final gate = BuzzPushAttemptGate(jitter: () => 1);
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

  /// Fails [attempt] once more and returns how long its retry took to fire.
  Future<Duration> failAndTimeRetry(
    WidgetTester tester,
    BuzzPushAttemptGate gate,
    String attempt,
  ) async {
    expect(gate.tryBegin(attempt), isTrue);
    var fired = false;
    gate.failed(attempt, retry: () => fired = true);
    var waited = Duration.zero;
    while (!fired) {
      await tester.pump(const Duration(milliseconds: 500));
      waited += const Duration(milliseconds: 500);
    }
    return waited;
  }

  testWidgets('repeated failures back off exponentially up to a cap', (
    tester,
  ) async {
    for (final jitter in [0.0, 1.0]) {
      final gate = BuzzPushAttemptGate(jitter: () => jitter);
      final delays = [
        for (var i = 0; i < 12; i++)
          await failAndTimeRetry(tester, gate, 'attempt'),
      ];
      gate.dispose();

      // Each delay lies between half and all of 5 s doubled per prior
      // failure, capped at 15 minutes.
      for (final (index, delay) in delays.indexed) {
        final ceiling = min(5000 * (1 << index), 15 * 60 * 1000);
        expect(
          delay.inMilliseconds,
          inInclusiveRange(ceiling ~/ 2, ceiling),
          reason: 'failure $index with jitter $jitter',
        );
      }
      expect(delays.last, lessThanOrEqualTo(const Duration(minutes: 15)));
      expect(delays.last, greaterThan(const Duration(minutes: 7)));
    }
  });

  testWidgets('a success ends the failure streak', (tester) async {
    final gate = BuzzPushAttemptGate(jitter: () => 1);
    addTearDown(gate.dispose);
    for (var i = 0; i < 4; i++) {
      await failAndTimeRetry(tester, gate, 'attempt');
    }

    expect(gate.tryBegin('attempt'), isTrue);
    gate.retryAfter('attempt', delay: Duration.zero, retry: () {});
    await tester.pump(const Duration(milliseconds: 1));

    expect(
      await failAndTimeRetry(tester, gate, 'attempt'),
      const Duration(seconds: 5),
    );
  });

  testWidgets('a registration success ends its failure streak', (tester) async {
    var failing = true;
    var registrations = 0;
    Widget bootstrap(String key) => BuzzPushRegistrationBootstrap(
      shouldRegister: true,
      attemptKey: key,
      startRegistration: () async {
        registrations += 1;
        if (failing) throw StateError('refused');
      },
      child: const SizedBox(),
    );

    // Community A fails long enough to back off past a minute.
    await tester.pumpWidget(bootstrap('a'));
    for (var i = 0; i < 5; i++) {
      await tester.pump(const Duration(minutes: 3));
    }
    expect(registrations, 6);
    failing = false;
    await tester.pump(const Duration(minutes: 15));
    expect(registrations, 7);

    // Switch to B and back. A's next failure retries within the first delay.
    await tester.pumpWidget(bootstrap('b'));
    expect(registrations, 8);
    failing = true;
    await tester.pumpWidget(bootstrap('a'));
    expect(registrations, 9);
    await tester.pump(const Duration(seconds: 5));
    await tester.pump();
    expect(registrations, 10);

    await tester.pumpWidget(const SizedBox());
  });

  test('a new pending tombstone changes the tombstone attempt', () {
    Community pending(String name, int generation) =>
        Community.create(name: name, relayUrl: 'wss://$name.example').copyWith(
          pushNotificationsEnabled: false,
          pushSubscriptionState: const BuzzPushLeaseSubscriptionState.desired(
            desired: [],
          ).withPendingTombstone(generation),
        );
    final a = pending('a', 4);
    final b = pending('b', 2);

    // A failing pass keeps its key, and so its backoff, until the work
    // changes; new work gets a new key and runs at once.
    expect(buzzPushTombstoneAttemptKey([a]), buzzPushTombstoneAttemptKey([a]));
    expect(
      buzzPushTombstoneAttemptKey([a, b]),
      isNot(buzzPushTombstoneAttemptKey([a])),
    );
    // Each pass advances the generation; that keeps the backoff.
    expect(
      buzzPushTombstoneAttemptKey([
        a.copyWith(
          pushSubscriptionState: a.pushSubscriptionState.withPendingTombstone(
            5,
          ),
        ),
      ]),
      buzzPushTombstoneAttemptKey([a]),
    );
  });

  testWidgets('a different attempt starts its own backoff', (tester) async {
    final gate = BuzzPushAttemptGate(jitter: () => 1);
    addTearDown(gate.dispose);
    for (var i = 0; i < 4; i++) {
      await failAndTimeRetry(tester, gate, 'old');
    }

    expect(
      await failAndTimeRetry(tester, gate, 'new'),
      const Duration(seconds: 5),
    );
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
