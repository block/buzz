import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:buzz/shared/community/community.dart';
import 'package:buzz/shared/community/community_provider.dart';
import 'package:buzz/shared/hosted/builderlab_account.dart';
import 'package:buzz/shared/hosted/hosted_relay_auth.dart';
import 'package:buzz/shared/push/push_bridge.dart';
import 'package:buzz/shared/push/push_lease_revocation_outbox.dart';
import 'package:buzz/shared/push/push_subscription.dart';
import 'package:buzz/shared/relay/relay_provider.dart';
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

  test(
    'revocation and tombstone sockets carry the hosted relay assertion',
    () async {
      // A hosted relay refuses the WebSocket upgrade without the assertion, so a
      // queued revocation or a removal tombstone would retry forever.
      // The test binding answers every HttpClient request with a 400, but this
      // test runs a real local relay.
      final overrides = HttpOverrides.current;
      HttpOverrides.global = null;
      addTearDown(() => HttpOverrides.global = overrides);
      final server = await HttpServer.bind(InternetAddress.loopbackIPv4, 0);
      addTearDown(() => server.close(force: true));
      final origin = 'ws://127.0.0.1:${server.port}';
      final upgrades = <HttpHeaders>[];
      server.listen((request) async {
        if (WebSocketTransformer.isUpgradeRequest(request)) {
          upgrades.add(request.headers);
          final socket = await WebSocketTransformer.upgrade(request);
          socket.add(jsonEncode(['AUTH', 'challenge']));
          socket.listen((message) {
            final frame = jsonDecode(message as String) as List<dynamic>;
            if (frame.first == 'AUTH' || frame.first == 'EVENT') {
              final event = frame[1] as Map<String, dynamic>;
              socket.add(jsonEncode(['OK', event['id'], true, '']));
            }
          });
          return;
        }
        request.response
          ..headers.contentType = ContentType('application', 'nostr+json')
          ..write(
            jsonEncode(_descriptorJson(origin, nostr.Keys.generate().public)),
          )
          ..close();
      });
      final auth = _AssertingRelayAuth();
      addTearDown(auth.dispose);

      await publishBuzzPushLeaseRevocation(
        BuzzPushLeaseRevocationRecord(
          relayUrl: 'http://127.0.0.1:${server.port}',
          relayOrigin: origin,
          memberPubkey: keys.public,
          nsec: keys.nsec,
          installationId: 'c' * 32,
          generation: 7,
          expiresAt: null,
          attemptCount: 0,
          nextAttemptAt: 0,
        ),
        relayAuth: auth,
      );

      expect(
        upgrades.single.value('nostr-federated-identity'),
        'Bearer hosted-token',
      );
      expect(auth.configs.single.baseUrl, 'http://127.0.0.1:${server.port}');
      expect(auth.configs.single.nsec, keys.nsec);
      // A removed community's failure must not hide the active one's recovery.
      expect(auth.backgrounds.single, isTrue);
      // A new socket rereads NIP-FI discovery, so a community that started
      // enforcing since its last read still gets the assertion.
      expect(auth.freshes.single, isTrue);

      // Removing the community publishes its tombstone the same way.
      debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
      addTearDown(() => debugDefaultTargetPlatformOverride = null);
      final messenger =
          TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger;
      messenger.setMockMethodCallHandler(const MethodChannel('buzz/push'), (
        call,
      ) async {
        expect(call.method, 'endpointGrants');
        return [
          {
            'relayOrigin': origin,
            'relayPubkey': 'a' * 64,
            'installationId': 'c' * 32,
            'endpointGrant': 'grant',
            'endpointHash': 'b' * 64,
            'endpointEpoch': 1,
            'generation': 1,
            'expiresAt': 1752624000,
          },
        ];
      });
      addTearDown(
        () => messenger.setMockMethodCallHandler(
          const MethodChannel('buzz/push'),
          null,
        ),
      );
      final container = ProviderContainer(
        overrides: [hostedRelayAuthProvider.overrideWithValue(auth)],
      );
      addTearDown(container.dispose);

      await container.read(communityPushLeaseDeactivatorProvider)(
        Community(
          id: 'removed',
          name: 'Removed',
          addedAt: DateTime.utc(2026),
          relayUrl: origin,
          pubkey: keys.public,
          nsec: keys.nsec,
        ),
        generation: 9,
      );

      expect(upgrades, hasLength(2));
      expect(
        upgrades.last.value('nostr-federated-identity'),
        'Bearer hosted-token',
      );
      expect(auth.configs.last.baseUrl, 'http://127.0.0.1:${server.port}');
      expect(auth.configs.last.nsec, keys.nsec);
      expect(auth.backgrounds.last, isTrue);
    },
  );

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

  test('removal without a grant retains cleanup through restart', () async {
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
    final first = _outbox(
      storage: storage,
      clock: clock,
      scheduler: scheduler,
      publisher: (_) async => throw StateError('offline'),
    );
    expect(
      await first.enqueueCommunity(community, readGrants: () async => []),
      isTrue,
    );
    await first.trigger();
    first.dispose();
    final pending = (await storage.loadAll()).single;
    expect(pending.installationId, community.pushLeaseInstallationId);
    expect(pending.expiresAt, isNull);
    clock.seconds = pending.nextAttemptAt;
    final published = <BuzzPushLeaseRevocationRecord>[];
    final restarted = _outbox(
      storage: storage,
      clock: clock,
      scheduler: scheduler,
      publisher: (record) async => published.add(record),
    );
    await restarted.trigger();
    expect(published.single.installationId, community.pushLeaseInstallationId);
    expect(published.single.generation, 7);
    expect(await storage.loadAll(), isEmpty);
    restarted.dispose();
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

class _AssertingRelayAuth extends HostedRelayAuth {
  _AssertingRelayAuth() : super(BuilderlabAccountClient(), enabled: false);

  final configs = <RelayConfig>[];
  final backgrounds = <bool>[];
  final freshes = <bool>[];

  @override
  Future<Map<String, String>> headers(
    RelayConfig config, {
    bool background = false,
    bool fresh = false,
  }) async {
    configs.add(config);
    backgrounds.add(background);
    freshes.add(fresh);
    return {'Nostr-Federated-Identity': 'Bearer hosted-token'};
  }
}

Map<String, dynamic> _descriptorJson(String origin, String relayPubkey) => {
  'supported_extensions': ['nip-er', 'buzz-push-v1'],
  'push': {
    'origin': origin,
    'keys': [
      {'id': 'relay-v1', 'pubkey': relayPubkey, 'current': true},
    ],
    'push_kinds': [9, 40002, 45001, 45003],
    'h_grammar': 'uuid-lowercase',
    'class_support': {
      'apns': ['default'],
    },
    'limitation': {
      'max_lease_ttl': 2592000,
      'max_leases_per_pubkey': 16,
      'max_subscriptions_per_lease': 16,
      'max_kinds': 16,
      'max_authors': 20,
      'max_h': 50,
      'max_tag_values': 20,
      'max_ignore': 8,
      'max_content_len': 65536,
      'max_plaintext_len': 32768,
      'max_endpoint_len': 4096,
      'max_string_len': 512,
    },
  },
};
