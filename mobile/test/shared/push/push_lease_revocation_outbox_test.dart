import 'dart:async';

import 'package:buzz/shared/community/community.dart';
import 'package:buzz/shared/push/push_bridge.dart';
import 'package:buzz/shared/push/push_lease_revocation_outbox.dart';
import 'package:buzz/shared/push/push_subscription.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:nostr/nostr.dart' as nostr;

import '../community/community_storage_test.dart';

void main() {
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
    expect(records, hasLength(1));
    expect(
      records.single.leaseAddress,
      '${keys.public}|wss://relay.example|${community.pushLeaseInstallationId}',
    );
    expect(records.single.generation, 6);
    expect(records.single.expiresAt, grant.expiresAt);
    expect(records.single.relayUrl, 'https://relay.example/');
  });

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
      (await storage.loadAll()).single.leaseAddress,
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

BuzzPushEndpointGrant _grant({required int expiresAt}) => BuzzPushEndpointGrant(
  relayOrigin: 'wss://relay.example',
  relayPubkey: 'a' * 64,
  installationId: '0' * 32,
  endpointGrant: 'opaque',
  endpointHash: 'b' * 64,
  appProfile: 'buzz-ios-dogfood',
  endpointEpoch: 1,
  generation: 1,
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
