import 'dart:async';

import 'package:flutter_secure_storage/flutter_secure_storage.dart';

import 'dart:convert';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:hooks_riverpod/misc.dart';
import 'package:buzz/shared/channels/channel_list_snapshot.dart';
import 'package:buzz/shared/channels/open_channel_directory_storage.dart';
import 'package:buzz/shared/channels/saved_message_store.dart';
import 'package:buzz/shared/crypto/event_signing.dart';
import 'package:buzz/shared/relay/nostr_models.dart';
import 'package:buzz/shared/theme/theme_provider.dart';
import 'package:shared_preferences/shared_preferences.dart';
import 'package:shared_preferences_platform_interface/shared_preferences_platform_interface.dart';
import 'package:buzz/shared/relay/media_disk_cache.dart';
import 'package:buzz/shared/relay/relay_provider.dart' show RelayConfig;
import 'package:buzz/shared/community/community.dart';
import 'package:buzz/shared/community/community_provider.dart';
import 'package:buzz/shared/community/community_storage.dart';
import 'package:buzz/shared/diagnostics/diagnostic_reporter.dart';
import 'package:buzz/shared/push/android_push_authority.dart';
import 'package:buzz/shared/push/dev_push_lease.dart' show BuzzPushLeaseState;
import 'package:buzz/shared/push/push_bridge.dart';
import 'package:buzz/shared/push/push_subscription.dart';
import 'package:buzz/shared/relay/signed_event_relay.dart'
    show SignedEventUnacknowledged;
import 'package:flutter/foundation.dart'
    show TargetPlatform, debugDefaultTargetPlatformOverride;
import 'package:nostr/nostr.dart' as nostr;

import '../../helpers/android_push_fakes.dart';
import 'community_storage_test.dart';

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  setUp(() => FlutterSecureStorage.setMockInitialValues({}));
  late FakeSecureStorage fakeSecure;
  late CommunityStorage communityStorage;
  late ProviderContainer container;
  late List<List<Community>> snapshots;
  late List<String> deactivatedCommunityIds;
  late List<int?> deactivationGenerations;
  late List<String> journaledCommunityIds;
  late int revocationTriggers;
  late CommunityPushLeaseRevocationTrigger revocationTrigger;
  late CommunityPushLeaseDeactivator deactivator;

  setUp(() {
    fakeSecure = FakeSecureStorage();
    communityStorage = CommunityStorage(secure: fakeSecure);
    snapshots = [];
    deactivatedCommunityIds = [];
    deactivationGenerations = [];
    journaledCommunityIds = [];
    revocationTriggers = 0;
    revocationTrigger = () async {
      revocationTriggers += 1;
    };
    deactivator = (community, {generation, deadline}) async {
      deactivatedCommunityIds.add(community.id);
      deactivationGenerations.add(generation);
    };
  });

  tearDown(() => container.dispose());

  ProviderContainer createContainer({
    List<Override> overrides = const [],
    CommunityPushLeaseRevocationEnqueuer? enqueueRevocation,
  }) {
    Future<void> writeSnapshot(List<Community> communities) async {
      snapshots.add(List.of(communities));
    }

    return ProviderContainer(
      overrides: [
        communityStorageProvider.overrideWithValue(communityStorage),
        communitySnapshotWriterProvider.overrideWithValue(writeSnapshot),
        communityPushLeaseDeactivatorProvider.overrideWithValue(deactivator),
        communityPushLeaseRevocationEnqueuerProvider.overrideWithValue(
          enqueueRevocation ??
              (community) async {
                journaledCommunityIds.add(community.id);
                return true;
              },
        ),
        communityPushLeaseRevocationTriggerProvider.overrideWithValue(
          revocationTrigger,
        ),
        ...overrides,
      ],
    );
  }

  group('CommunityListNotifier', () {
    test('loads empty list initially', () async {
      container = createContainer();
      final communities = await container.read(communityListProvider.future);
      expect(communities, isEmpty);
      expect(snapshots, [isEmpty]);
    });

    test('exports migrated communities on startup', () async {
      final community = Community.create(
        name: 'Migrated',
        relayUrl: 'https://migrated.example.com',
        nsec: nostr.Keys.generate().nsec,
      );
      // Seed legacy storage to exercise the same migration path as an app
      // upgrade.
      fakeSecure['buzz_workspaces'] = jsonEncode([community.toJson()]);

      container = createContainer();
      await container.read(communityListProvider.future);

      expect(snapshots.single.single.id, community.id);
      expect(fakeSecure['buzz_workspaces'], isNull);
    });

    test('skips an unchanged snapshot after provider invalidation', () async {
      final community = Community.create(
        name: 'Stored',
        relayUrl: 'https://stored.example.com',
        nsec: nostr.Keys.generate().nsec,
      );
      await communityStorage.save(community);
      container = createContainer();

      await container.read(communityListProvider.future);
      container.invalidate(communityListProvider);
      await container.read(communityListProvider.future);

      expect(snapshots, hasLength(1));
    });

    test(
      'push notifications default on and opt-out survives restart',
      () async {
        final community = Community.create(
          name: 'Test',
          relayUrl: 'https://test.example.com',
        );
        await communityStorage.save(community);
        container = createContainer();
        await container.read(communityListProvider.future);
        expect(
          (await container.read(
            communityListProvider.future,
          )).single.pushNotificationsEnabled,
          isTrue,
        );

        await container
            .read(communityListProvider.notifier)
            .setPushNotificationsEnabled(community.id, false);
        container.dispose();
        container = createContainer();

        expect(
          (await container.read(
            communityListProvider.future,
          )).single.pushNotificationsEnabled,
          isFalse,
        );
      },
    );

    test(
      'lease retry reserves beyond a locally unaccepted generation',
      () async {
        final subscription = BuzzPushSubscription(
          filter: BuzzPushFilter(kinds: const [9], pTags: ['a' * 64]),
          notificationClass: 'default',
        );
        final subscriptionState = BuzzPushLeaseSubscriptionState.desired(
          desired: [subscription],
        ).withAccepted(subscriptions: [subscription], generation: 4);
        final community =
            Community.create(
              name: 'Test',
              relayUrl: 'https://test.example.com',
            ).copyWith(
              pushNotificationsEnabled: true,
              pushSubscriptionState: subscriptionState,
            );
        await communityStorage.save(community);
        container = createContainer();
        await container.read(communityListProvider.future);
        final notifier = container.read(communityListProvider.notifier);

        expect(await notifier.reservePushLeaseGeneration(community.id), 5);
        expect(await notifier.reservePushLeaseGeneration(community.id), 6);
        final stored = (await communityStorage.loadAll()).single;
        expect(stored.pushSubscriptionState.acceptedGeneration, 4);
        expect(stored.pushSubscriptionState.generationCursor, 6);
      },
    );

    test('older lease success cannot regress accepted generation', () async {
      final subscription = BuzzPushSubscription(
        filter: BuzzPushFilter(kinds: const [9], pTags: ['a' * 64]),
        notificationClass: 'default',
      );
      final community =
          Community.create(
            name: 'Test',
            relayUrl: 'https://test.example.com',
          ).copyWith(
            pushNotificationsEnabled: true,
            pushSubscriptionState: BuzzPushLeaseSubscriptionState.desired(
              desired: [subscription],
            ).withAccepted(subscriptions: [subscription], generation: 6),
          );
      await communityStorage.save(community);
      container = createContainer();
      await container.read(communityListProvider.future);
      final notifier = container.read(communityListProvider.notifier);

      expect(
        await notifier.markPushLeaseAccepted(
          community.id,
          subscriptions: const [],
          generation: 5,
        ),
        isFalse,
      );

      final stored = (await communityStorage.loadAll()).single;
      expect(stored.pushSubscriptionState.acceptedGeneration, 6);
      expect(
        stored.pushSubscriptionState.accepted!.single.toJson(),
        subscription.toJson(),
      );
    });

    test('lease success cannot accept stale desired subscriptions', () async {
      final original = BuzzPushSubscription(
        filter: BuzzPushFilter(kinds: const [9], pTags: ['a' * 64]),
        notificationClass: 'default',
      );
      final replacement = BuzzPushSubscription(
        filter: BuzzPushFilter(kinds: const [9], pTags: ['b' * 64]),
        notificationClass: 'default',
      );
      final community =
          Community.create(
            name: 'Test',
            relayUrl: 'https://test.example.com',
          ).copyWith(
            pushNotificationsEnabled: true,
            pushSubscriptionState: BuzzPushLeaseSubscriptionState.desired(
              desired: [original],
            ),
          );
      await communityStorage.save(community);
      container = createContainer();
      await container.read(communityListProvider.future);
      final notifier = container.read(communityListProvider.notifier);

      final generation = await notifier.reservePushLeaseGeneration(
        community.id,
      );
      await notifier.updateDesiredPushSubscriptions(community.id, [
        replacement,
      ]);

      expect(
        await notifier.markPushLeaseAccepted(
          community.id,
          subscriptions: [original],
          generation: generation,
        ),
        isFalse,
      );

      final stored = (await communityStorage.loadAll()).single;
      expect(stored.pushSubscriptionState.accepted, isNull);
      expect(
        stored.pushSubscriptionState.desired.single.toJson(),
        replacement.toJson(),
      );
    });

    test('opt-out tombstones an in-flight first publication', () async {
      final subscription = BuzzPushSubscription(
        filter: BuzzPushFilter(kinds: const [9], pTags: ['a' * 64]),
        notificationClass: 'default',
      );
      final community =
          Community.create(
            name: 'Test',
            relayUrl: 'https://test.example.com',
          ).copyWith(
            pushNotificationsEnabled: true,
            pushSubscriptionState: BuzzPushLeaseSubscriptionState.desired(
              desired: [subscription],
            ),
          );
      await communityStorage.save(community);
      container = createContainer();
      await container.read(communityListProvider.future);
      final notifier = container.read(communityListProvider.notifier);

      expect(await notifier.reservePushLeaseGeneration(community.id), 1);
      await notifier.setPushNotificationsEnabled(community.id, false);
      await notifier.markPushLeaseAccepted(
        community.id,
        subscriptions: [subscription],
        generation: 1,
      );

      final stored = (await communityStorage.loadAll()).single;
      expect(stored.pushNotificationsEnabled, isFalse);
      expect(stored.pushSubscriptionState.acceptedGeneration, 2);
      expect(stored.pushSubscriptionState.generationCursor, 2);
      expect(stored.pushSubscriptionState.pendingTombstoneGeneration, isNull);
      expect(deactivationGenerations, [2]);
    });

    test('opt-out persists first and publishes a higher tombstone', () async {
      final subscription = BuzzPushSubscription(
        filter: BuzzPushFilter(kinds: const [9], pTags: ['a' * 64]),
        notificationClass: 'default',
      );
      final community =
          Community.create(
            name: 'Test',
            relayUrl: 'https://test.example.com',
          ).copyWith(
            pushNotificationsEnabled: true,
            pushSubscriptionState: BuzzPushLeaseSubscriptionState.desired(
              desired: [subscription],
            ).withAccepted(subscriptions: [subscription], generation: 7),
          );
      await communityStorage.save(community);
      container = createContainer();
      await container.read(communityListProvider.future);
      final notifier = container.read(communityListProvider.notifier);

      await notifier.setPushNotificationsEnabled(community.id, false);

      final stored = (await communityStorage.loadAll()).single;
      expect(stored.pushNotificationsEnabled, isFalse);
      expect(stored.pushSubscriptionState.generationCursor, 8);
      expect(stored.pushSubscriptionState.acceptedGeneration, 8);
      expect(stored.pushSubscriptionState.pendingTombstoneGeneration, isNull);
      expect(deactivatedCommunityIds, [community.id]);
      expect(deactivationGenerations, [8]);

      container.dispose();
      container = createContainer();
      expect(
        (await container.read(
          communityListProvider.future,
        )).single.pushNotificationsEnabled,
        isFalse,
      );
    });

    test(
      'a refused opt-out restores push, and Retry after restart uses a newer generation',
      () async {
        var failTombstone = true;
        deactivator = (community, {generation, deadline}) async {
          deactivatedCommunityIds.add(community.id);
          deactivationGenerations.add(generation);
          if (failTombstone) {
            throw StateError('injected tombstone failure');
          }
        };
        final subscription = BuzzPushSubscription(
          filter: BuzzPushFilter(kinds: const [9], pTags: ['a' * 64]),
          notificationClass: 'default',
        );
        final community =
            Community.create(
              name: 'Test',
              relayUrl: 'https://test.example.com',
            ).copyWith(
              pushNotificationsEnabled: true,
              pushSubscriptionState: BuzzPushLeaseSubscriptionState.desired(
                desired: [subscription],
              ).withAccepted(subscriptions: [subscription], generation: 7),
            );
        await communityStorage.save(community);
        container = createContainer();
        await container.read(communityListProvider.future);

        await container
            .read(communityListProvider.notifier)
            .setPushNotificationsEnabled(community.id, false);

        // The relay never stored it, so the confirmed lease is restored and
        // the spent generation stays burned.
        var stored = (await communityStorage.loadAll()).single;
        expect(stored.pushNotificationsEnabled, isTrue);
        expect(stored.pushSubscriptionState.acceptedGeneration, 7);
        expect(stored.pushSubscriptionState.generationCursor, 8);
        expect(stored.pushSubscriptionState.pendingTombstoneGeneration, isNull);
        expect(deactivationGenerations, [8]);

        container.dispose();
        failTombstone = false;
        container = createContainer();
        await container.read(communityListProvider.future);
        await container
            .read(communityListProvider.notifier)
            .retryPushOptOut(community.id);

        stored = (await communityStorage.loadAll()).single;
        expect(stored.pushNotificationsEnabled, isFalse);
        expect(stored.pushSubscriptionState.acceptedGeneration, 9);
        expect(stored.pushSubscriptionState.generationCursor, 9);
        expect(stored.pushSubscriptionState.pendingTombstoneGeneration, isNull);
        expect(deactivationGenerations, [8, 9]);
      },
    );

    test(
      'a refused opt-out with no confirmed lease stays off and guarded',
      () async {
        Object failure = StateError('injected refusal');
        deactivator = (community, {generation, deadline}) async {
          deactivationGenerations.add(generation);
          throw failure;
        };
        final subscription = BuzzPushSubscription(
          filter: BuzzPushFilter(kinds: const [9], pTags: ['a' * 64]),
          notificationClass: 'default',
        );
        // Turned back on after an earlier opt-out, but the relay never
        // confirmed the new lease: the confirmed setting is off.
        final community =
            Community.create(
              name: 'Test',
              relayUrl: 'https://test.example.com',
            ).copyWith(
              pushNotificationsEnabled: true,
              pushSubscriptionState: BuzzPushLeaseSubscriptionState.desired(
                desired: [subscription],
                generationCursor: 8,
              ),
            );
        await communityStorage.save(community);
        final leaseRead = Completer<BuzzPushLeaseState?>();
        container = createContainer(
          overrides: [
            communityPushLeaseReaderProvider.overrideWithValue(
              (_) => leaseRead.future,
            ),
          ],
        );
        await container.read(communityListProvider.future);

        await container
            .read(communityListProvider.notifier)
            .setPushNotificationsEnabled(community.id, false);

        final stored = (await communityStorage.loadAll()).single;
        expect(stored.pushNotificationsEnabled, isFalse);
        expect(stored.pushSubscriptionState.pendingTombstoneGeneration, 9);
        expect(deactivationGenerations, [9]);
        expect(
          container.read(pushOptOutStatusProvider)[community.id],
          PushOptOutStatus.failed,
        );

        // A lease read starts, then a Retry publishes generation 10 and its
        // answer is lost. The read predates that tombstone, so its answer
        // (still active) can't settle it: push stays off and guarded.
        final notifier = container.read(communityListProvider.notifier);
        final reconcile = notifier.reconcilePushOptOut(community.id);
        failure = SignedEventUnacknowledged(TimeoutException('no OK'));
        await notifier.retryPushOptOut(community.id);
        leaseRead.complete((active: true, generation: 8));
        await reconcile;
        final retried = (await communityStorage.loadAll()).single;
        expect(retried.pushNotificationsEnabled, isFalse);
        expect(retried.pushSubscriptionState.pendingTombstoneGeneration, 10);
        expect(deactivationGenerations, [9, 10]);
        expect(
          container.read(pushOptOutStatusProvider)[community.id],
          PushOptOutStatus.failed,
        );
      },
    );

    test(
      'a Retry that fails before sending keeps an opt-out whose OK was lost',
      () async {
        Object failure = SignedEventUnacknowledged(TimeoutException('no OK'));
        deactivator = (community, {generation, deadline}) async {
          deactivationGenerations.add(generation);
          throw failure;
        };
        final subscription = BuzzPushSubscription(
          filter: BuzzPushFilter(kinds: const [9], pTags: ['a' * 64]),
          notificationClass: 'default',
        );
        final community =
            Community.create(
              name: 'Test',
              relayUrl: 'https://test.example.com',
            ).copyWith(
              pushNotificationsEnabled: true,
              pushSubscriptionState: BuzzPushLeaseSubscriptionState.desired(
                desired: [subscription],
              ).withAccepted(subscriptions: [subscription], generation: 7),
            );
        await communityStorage.save(community);
        container = createContainer();
        await container.read(communityListProvider.future);
        final notifier = container.read(communityListProvider.notifier);

        // The relay may hold generation 8: its OK never came back.
        await notifier.setPushNotificationsEnabled(community.id, false);
        expect(
          (await communityStorage.loadAll())
              .single
              .pushSubscriptionState
              .pendingTombstoneGeneration,
          8,
        );

        // Still offline, the Retry fails before sending anything. Push must
        // stay off: turning it back on would let the next sync publish an
        // active lease over the opt-out the relay may already hold.
        failure = StateError('offline');
        await notifier.retryPushOptOut(community.id);

        final stored = (await communityStorage.loadAll()).single;
        expect(stored.pushNotificationsEnabled, isFalse);
        expect(stored.pushSubscriptionState.pendingTombstoneGeneration, 9);
        expect(deactivationGenerations, [8, 9]);
        expect(
          container.read(pushOptOutStatusProvider)[community.id],
          PushOptOutStatus.failed,
        );
      },
    );

    test(
      'a failed save after the relay accepts an opt-out keeps it pending',
      () async {
        final storage = _FailingSaveStorage(fakeSecure);
        communityStorage = storage;
        deactivator = (community, {generation, deadline}) async {
          deactivationGenerations.add(generation);
          // The relay answered OK; recording that locally is what fails.
          storage.failNextSave = true;
        };
        final subscription = BuzzPushSubscription(
          filter: BuzzPushFilter(kinds: const [9], pTags: ['a' * 64]),
          notificationClass: 'default',
        );
        final community =
            Community.create(
              name: 'Test',
              relayUrl: 'https://test.example.com',
            ).copyWith(
              pushNotificationsEnabled: true,
              pushSubscriptionState: BuzzPushLeaseSubscriptionState.desired(
                desired: [subscription],
              ).withAccepted(subscriptions: [subscription], generation: 7),
            );
        await communityStorage.save(community);
        container = createContainer();
        await container.read(communityListProvider.future);

        await container
            .read(communityListProvider.notifier)
            .setPushNotificationsEnabled(community.id, false);

        final stored = (await communityStorage.loadAll()).single;
        expect(stored.pushNotificationsEnabled, isFalse);
        expect(stored.pushSubscriptionState.pendingTombstoneGeneration, 8);
        expect(stored.pushSubscriptionState.acceptedGeneration, 7);
        expect(deactivationGenerations, [8]);
        expect(
          container.read(pushOptOutStatusProvider)[community.id],
          PushOptOutStatus.failed,
        );
      },
    );

    test(
      'opt-out after a gateway change still tombstones with the saved lease id',
      () async {
        // The test binding answers every HttpClient request with a 400, but
        // this test runs a real local relay.
        final overrides = HttpOverrides.current;
        HttpOverrides.global = null;
        addTearDown(() => HttpOverrides.global = overrides);
        debugDefaultTargetPlatformOverride = TargetPlatform.android;
        addTearDown(() => debugDefaultTargetPlatformOverride = null);
        final server = await HttpServer.bind(InternetAddress.loopbackIPv4, 0);
        addTearDown(() => server.close(force: true));
        final origin = 'ws://127.0.0.1:${server.port}';
        final tombstones = <Map<String, dynamic>>[];
        server.listen((request) async {
          if (WebSocketTransformer.isUpgradeRequest(request)) {
            final socket = await WebSocketTransformer.upgrade(request);
            socket.add(jsonEncode(['AUTH', 'challenge']));
            socket.listen((message) {
              final frame = jsonDecode(message as String) as List<dynamic>;
              final event = frame[1] as Map<String, dynamic>;
              if (frame.first == 'EVENT') tombstones.add(event);
              socket.add(jsonEncode(['OK', event['id'], true, '']));
            });
            return;
          }
          request.response
            ..headers.contentType = ContentType('application', 'nostr+json')
            ..write(jsonEncode(_descriptorJson(origin)))
            ..close();
        });

        // The phone enrolled with one gateway, then the app was built for
        // another: that gateway's state reads empty, so no grant names the
        // relay, but the community remembers its lease's installation id.
        final store = _MemoryStore();
        final keystore = FakeAndroidKeystore();
        final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
        final oldAuthority = AndroidPushAuthority(
          keystore: keystore,
          gateway: fakeGatewayClient(FakePushGateway(nowSeconds: () => now)),
          store: store,
          sleep: (_) async {},
        );
        await oldAuthority.enroll(
          relayOrigin: origin,
          relayPubkey: '2' * 64,
          endpoint: 'token-1',
          stillWanted: () => true,
          desiredRelays: () => {origin},
        );
        debugAndroidPushAuthority = AndroidPushAuthority(
          keystore: keystore,
          gateway: fakeGatewayClient(
            FakePushGateway(nowSeconds: () => now),
            baseUrl: 'https://other-push.example',
          ),
          store: store,
          sleep: (_) async {},
        );
        addTearDown(() => debugAndroidPushAuthority = null);
        expect(await readBuzzPushEndpointGrants(), isEmpty);

        final keys = nostr.Keys.generate();
        final subscription = BuzzPushSubscription(
          filter: BuzzPushFilter(kinds: const [9], pTags: ['a' * 64]),
          notificationClass: 'default',
        );
        final community =
            Community.create(
              name: 'Test',
              relayUrl: 'http://127.0.0.1:${server.port}',
              nsec: keys.nsec,
            ).copyWith(
              pushNotificationsEnabled: true,
              pushLeaseInstallationId: 'd' * 32,
              pushSubscriptionState: BuzzPushLeaseSubscriptionState.desired(
                desired: [subscription],
              ).withAccepted(subscriptions: [subscription], generation: 7),
            );
        // The real deactivator, unlike the injected one createContainer uses.
        await communityStorage.save(community);
        container = ProviderContainer(
          overrides: [
            communityStorageProvider.overrideWithValue(communityStorage),
            communitySnapshotWriterProvider.overrideWithValue(
              (communities) async {},
            ),
            communityPushLeaseRevocationEnqueuerProvider.overrideWithValue(
              (community) async => true,
            ),
            communityPushLeaseRevocationTriggerProvider.overrideWithValue(
              () async {},
            ),
          ],
        );
        await container.read(communityListProvider.future);
        final notifier = container.read(communityListProvider.notifier);

        await notifier.setPushNotificationsEnabled(community.id, false);

        final stored = (await communityStorage.loadAll()).single;
        expect(stored.pushSubscriptionState.pendingTombstoneGeneration, isNull);
        final tombstone = tombstones.single;
        expect(
          (tombstone['tags'] as List).any(
            (tag) => (tag as List).contains('d' * 32),
          ),
          isTrue,
        );
      },
    );

    test('removeCommunity removes from list', () async {
      final ws = Community.create(
        name: 'Test',
        relayUrl: 'https://test.example.com',
      );
      await communityStorage.save(ws);
      container = createContainer();
      await container.read(communityListProvider.future);
      await container
          .read(communityListProvider.notifier)
          .removeCommunity(ws.id);

      final communities = await container.read(communityListProvider.future);
      expect(communities, isEmpty);
      expect(journaledCommunityIds, [ws.id]);
      expect(deactivatedCommunityIds, isEmpty);
      expect(revocationTriggers, 1);
    });

    test(
      'removal deletes the channel list snapshot and retires writers',
      () async {
        SharedPreferences.setMockInitialValues({});
        final prefs = await SharedPreferences.getInstance();
        final keys = nostr.Keys.generate();
        final community = Community.create(
          name: 'Cached',
          relayUrl: 'wss://cached.example.com:443',
          pubkey: 'incorrect-declared-key',
          nsec: '  ${keys.nsec}  ',
        );
        final otherScope = Community.create(
          name: 'Other',
          relayUrl: 'https://other.example.com',
          nsec: keys.nsec,
        );
        final snapshots = ChannelListSnapshotStorage(prefs);
        const snapshot = ChannelListSnapshot(channels: [], hiddenDmIds: {});
        await communityStorage.save(community);
        await communityStorage.save(otherScope);
        // Use the actual provider writer origin, not the raw persisted URL.
        final writerRelay = RelayConfig(baseUrl: community.relayUrl).baseUrl;
        // A refresh captured its token before removal and writes after it.
        final inFlight = snapshots.capture(keys.public, writerRelay);
        await snapshots.write(inFlight, snapshot);
        await snapshots.write(
          snapshots.capture(keys.public, otherScope.relayUrl),
          snapshot,
        );
        final savedRoot = await Directory.systemTemp.createTemp('saved_root');
        addTearDown(() => savedRoot.delete(recursive: true));
        container = createContainer(
          overrides: [
            savedPrefsProvider.overrideWithValue(prefs),
            savedMessageRootProvider.overrideWith((ref) async => savedRoot),
          ],
        );
        final notifier = container.read(communityListProvider.notifier);
        await container.read(communityListProvider.future);
        // Saved messages are scoped by the key the nsec signs with, not the
        // declared pubkey.
        final saved = container.read(savedMessageStoreProvider);
        final savedScope = (communityId: community.id, pubkey: keys.public);
        final otherSavedScope = (
          communityId: otherScope.id,
          pubkey: keys.public,
        );
        await _saveMessage(saved, savedScope, keys.secret);
        await _saveMessage(saved, otherSavedScope, keys.secret);
        final inFlightSave = saved.capture(savedScope, _savedPage);
        expect(await _readSaved(saved, savedScope), hasLength(1));

        await notifier.removeCommunity(community.id);

        expect(snapshots.read(keys.public, writerRelay), isNull);
        expect(await snapshots.write(inFlight, snapshot), isFalse);
        expect(snapshots.read(keys.public, writerRelay), isNull);
        expect(snapshots.read(keys.public, otherScope.relayUrl), isNotNull);
        await saved.save(inFlightSave, [
          _savedMessage(keys.secret),
        ], relayAuthor: null);
        expect(await _readSaved(saved, savedScope), isNull);
        expect(await _readSaved(saved, otherSavedScope), hasLength(1));
      },
    );

    test('startup does not wait for orphan disk deletes', () async {
      SharedPreferences.setMockInitialValues({});
      final store = _SnapshotStore();
      SharedPreferencesStorePlatform.instance = store;
      final prefs = await SharedPreferences.getInstance();
      final cache = ChannelListSnapshotStorage(prefs);
      const snapshot = ChannelListSnapshot(channels: [], hiddenDmIds: {});
      for (final id in ['orphan-a', 'orphan-b']) {
        await cache.write(
          cache.capture(id, 'https://orphan.example'),
          snapshot,
        );
      }
      final gate = store.removeGate = Completer<void>();
      container = createContainer(
        overrides: [savedPrefsProvider.overrideWithValue(prefs)],
      );
      expect(await container.read(communityListProvider.future), isEmpty);
      // All old deletions are submitted before new scopes can start writers.
      expect(store.removeAttempts, 2);
      final token = cache.capture('orphan-b', 'https://orphan.example');
      expect(await cache.write(token, snapshot), isTrue);
      gate.complete();
      await Future<void>.delayed(Duration.zero);
      expect(await cache.write(token, snapshot), isTrue);
      expect((await store.getAll()).values, [
        prefs.getString(
          ChannelListSnapshotStorage.key('orphan-b', 'https://orphan.example'),
        ),
      ]);
    });

    test(
      'startup and removal discard orphan snapshots and image caches',
      () async {
        final mediaRoot = await Directory.systemTemp.createTemp('media_root');
        addTearDown(() => mediaRoot.delete(recursive: true));
        Future<Directory> seedMedia(Directory directory) async {
          await File('${directory.path}/image').create(recursive: true);
          return directory;
        }

        Future<File> seedDirectory(String communityId, String signer) async {
          final file = OpenChannelDirectoryStorage.fileFor(
            mediaRoot,
            communityId,
            signer,
          );
          await file.create(recursive: true);
          return file;
        }

        // Media sweeps run in the background.
        Future<bool> eventuallyExists(FileSystemEntity directory) async {
          for (var i = 0; i < 50 && await directory.exists(); i++) {
            await Future<void>.delayed(const Duration(milliseconds: 10));
          }
          return directory.exists();
        }

        SharedPreferences.setMockInitialValues({});
        final prefs = await SharedPreferences.getInstance();
        final cache = ChannelListSnapshotStorage(prefs);
        const snapshot = ChannelListSnapshot(channels: [], hiddenDmIds: {});
        final oldKeys = nostr.Keys.generate();
        final community = Community.create(
          name: 'Stored',
          relayUrl: 'wss://cached.example.com:443',
          nsec: oldKeys.nsec,
        );
        await communityStorage.save(community);
        final oldToken = cache.capture(oldKeys.public, community.relayUrl);
        final restoredToken = cache.capture(
          'restored',
          'https://orphan.example',
        );
        await cache.write(oldToken, snapshot);
        await cache.write(restoredToken, snapshot);
        await prefs.setString('unrelated', 'keep');
        final oldMedia = await seedMedia(
          MediaDiskCache.directoryFor(mediaRoot, community.id, oldKeys.public),
        );
        final orphanMedia = await seedMedia(
          MediaDiskCache.communityDirectory(mediaRoot, 'restored'),
        );
        final oldDirectory = await seedDirectory(community.id, oldKeys.public);
        container = createContainer(
          overrides: [
            savedPrefsProvider.overrideWithValue(prefs),
            mediaCacheRootProvider.overrideWith((ref) async => mediaRoot),
          ],
        );
        await container.read(communityListProvider.future);
        expect(cache.read(oldKeys.public, community.relayUrl), isNotNull);
        expect(cache.read('restored', 'https://orphan.example'), isNull);
        expect(await cache.write(restoredToken, snapshot), isFalse);
        expect(await eventuallyExists(orphanMedia), isFalse);
        expect(await oldMedia.exists(), isTrue);
        expect(await oldDirectory.exists(), isTrue);

        await communityStorage.saveActiveId(community.id);
        await container
            .read(communityListProvider.notifier)
            .removeCommunity(community.id);
        expect(await communityStorage.loadAll(), isEmpty);
        expect(prefs.getKeys(), {'unrelated'});
        expect(await cache.write(oldToken, snapshot), isFalse);
        expect(await eventuallyExists(oldMedia), isFalse);
        expect(await eventuallyExists(oldDirectory), isFalse);
        expect(
          await eventuallyExists(
            MediaDiskCache.communityDirectory(mediaRoot, community.id),
          ),
          isFalse,
        );

        // A backup restored without secure community records is swept at startup.
        container.dispose();
        await prefs.setString(
          ChannelListSnapshotStorage.key('restored', 'https://orphan.example'),
          '{"v":1,"channels":[],"hidden_dm_ids":[]}',
        );
        container = createContainer(
          overrides: [savedPrefsProvider.overrideWithValue(prefs)],
        );
        await container.read(communityListProvider.future);
        expect(prefs.getKeys(), {'unrelated'});
      },
    );

    test('startup and removal purge saved messages from disk', () async {
      final root = await Directory.systemTemp.createTemp('saved_root');
      addTearDown(() => root.delete(recursive: true));
      final oldKeys = nostr.Keys.generate();
      final community = Community.create(
        name: 'Stored',
        relayUrl: 'wss://cached.example.com:443',
        nsec: oldKeys.nsec,
      );
      await communityStorage.save(community);
      final oldSaved = (communityId: community.id, pubkey: oldKeys.public);
      final orphanSaved = (communityId: 'restored', pubkey: oldKeys.public);
      // A previous launch saved this account's messages and those of a
      // community removed while the app was not running.
      final launch = SavedMessageStore(
        root: () async => root,
        diagnostics: const NoopDiagnosticReporter(),
      );
      await _saveMessage(launch, oldSaved, oldKeys.secret);
      await _saveMessage(launch, orphanSaved, oldKeys.secret);
      late SavedMessageTicket removalInFlight;
      container = createContainer(
        overrides: [savedMessageRootProvider.overrideWith((ref) async => root)],
        enqueueRevocation: (_) async {
          // The active account is still published during revocation journaling.
          removalInFlight = container
              .read(savedMessageStoreProvider)
              .capture(oldSaved, _savedPage);
          return true;
        },
      );
      await container.read(communityListProvider.future);
      final saved = container.read(savedMessageStoreProvider);
      // The store's queue runs the startup sweep before these reads.
      expect(await _readSaved(saved, orphanSaved), isNull);
      expect(await _readSaved(saved, oldSaved), hasLength(1));

      await container.read(activeCommunityProvider.future);
      expect(container.read(savedMessageScopeProvider), oldSaved);

      // The active provider still holds the removed account when the list
      // publishes; the scope must already be gone.
      final scopesDuringRefresh = <SavedMessageScope?>[];
      container.listen(communityListProvider, (_, next) {
        if (next.value?.isEmpty == true &&
            container.read(activeCommunityProvider).value?.id == community.id) {
          scopesDuringRefresh.add(container.read(savedMessageScopeProvider));
        }
      });
      await container
          .read(communityListProvider.notifier)
          .removeCommunity(community.id);
      expect(scopesDuringRefresh, [isNull]);
      await saved.save(removalInFlight, [
        _savedMessage(oldKeys.secret),
      ], relayAuthor: null);
      expect(await _readSaved(saved, oldSaved), isNull);
      // No database or journal survives.
      expect(Directory('${root.path}/saved_messages').listSync(), isEmpty);
    });

    for (final failure in ['false', 'throw']) {
      test(
        'cache delete $failure cannot block sign-out; cleanup retries',
        () async {
          SharedPreferences.setMockInitialValues({});
          final store = _SnapshotStore();
          SharedPreferencesStorePlatform.instance = store;
          final prefs = await SharedPreferences.getInstance();
          final keys = nostr.Keys.generate();
          final community = Community.create(
            name: 'Cached',
            relayUrl: 'https://cache.example',
            nsec: keys.nsec,
          );
          await communityStorage.save(community);
          await communityStorage.saveActiveId(community.id);
          final cache = ChannelListSnapshotStorage(prefs);
          final token = cache.capture(keys.public, community.relayUrl);
          const snapshot = ChannelListSnapshot(channels: [], hiddenDmIds: {});
          await cache.write(token, snapshot);
          container = createContainer(
            overrides: [savedPrefsProvider.overrideWithValue(prefs)],
            enqueueRevocation: (community) async {
              expect(store.removeAttempts, greaterThan(0));
              journaledCommunityIds.add(community.id);
              return true;
            },
          );
          await container.read(communityListProvider.future);
          store.failure = failure;
          await container
              .read(communityListProvider.notifier)
              .removeCommunity(community.id);
          expect(await communityStorage.loadAll(), isEmpty);
          expect(await communityStorage.loadActiveId(), isNull);
          expect(journaledCommunityIds, [community.id]);
          expect(revocationTriggers, 1);
          expect(await cache.write(token, snapshot), isFalse);
          expect(
            (await store.getAll()).keys,
            isNotEmpty,
          ); // Disk deletion failed.
          store.failure = null;
          container.invalidate(communityListProvider);
          await container.read(communityListProvider.future);
          expect(await store.getAll(), isEmpty);
        },
      );
    }

    test('remote tombstone attempt cannot block local removal', () async {
      final remoteAttempt = Completer<void>();
      revocationTrigger = () {
        revocationTriggers += 1;
        return remoteAttempt.future;
      };
      final community = Community.create(
        name: 'Test',
        relayUrl: 'https://test.example.com',
      );
      await communityStorage.save(community);
      container = createContainer();
      await container.read(communityListProvider.future);
      final notifier = container.read(communityListProvider.notifier);

      await notifier.removeCommunity(community.id);

      expect(await communityStorage.loadAll(), isEmpty);
      expect(journaledCommunityIds, [community.id]);
      expect(revocationTriggers, 1);
      remoteAttempt.complete();
    });

    test('journal persistence failure keeps community credentials', () async {
      final community = Community.create(
        name: 'Test',
        relayUrl: 'https://test.example.com',
      );
      await communityStorage.save(community);
      container = ProviderContainer(
        overrides: [
          communityStorageProvider.overrideWithValue(communityStorage),
          communitySnapshotWriterProvider.overrideWithValue((_) async {}),
          communityPushLeaseRevocationEnqueuerProvider.overrideWithValue((
            _,
          ) async {
            throw StateError('secure storage unavailable');
          }),
          communityPushLeaseRevocationTriggerProvider.overrideWithValue(
            () async {},
          ),
        ],
      );
      await container.read(communityListProvider.future);
      final notifier = container.read(communityListProvider.notifier);

      await expectLater(
        notifier.removeCommunity(community.id),
        throwsStateError,
      );

      expect(
        (await communityStorage.loadAll()).map((item) => item.id),
        contains(community.id),
      );
    });

    test(
      'waits for transition teardown before removing active community',
      () async {
        final ws1 = Community.create(
          name: 'One',
          relayUrl: 'https://one.example.com',
        );
        final ws2 = Community.create(
          name: 'Two',
          relayUrl: 'https://two.example.com',
        );
        await communityStorage.save(ws1);
        await communityStorage.save(ws2);
        container = createContainer();
        await container.read(communityListProvider.future);
        final notifier = container.read(communityListProvider.notifier);
        await notifier.switchCommunity(ws1.id);
        final teardown = Completer<void>();
        container
            .read(communityTransitionProvider)
            .register(() => teardown.future);

        final removing = notifier.removeCommunity(ws1.id);
        await Future<void>.delayed(Duration.zero);

        expect(await communityStorage.loadActiveId(), ws1.id);
        expect(
          (await communityStorage.loadAll()).map((community) => community.id),
          contains(ws1.id),
        );
        teardown.complete();
        await removing;
        expect(await communityStorage.loadActiveId(), ws2.id);
        expect(
          (await communityStorage.loadAll()).map((community) => community.id),
          isNot(contains(ws1.id)),
        );
      },
    );

    test('waits for transition teardown before updating active ID', () async {
      final ws1 = Community.create(
        name: 'One',
        relayUrl: 'https://one.example.com',
      );
      final ws2 = Community.create(
        name: 'Two',
        relayUrl: 'https://two.example.com',
      );
      await communityStorage.save(ws1);
      await communityStorage.save(ws2);
      container = createContainer();
      await container.read(communityListProvider.future);
      final notifier = container.read(communityListProvider.notifier);
      await notifier.switchCommunity(ws1.id);
      final teardown = Completer<void>();
      container
          .read(communityTransitionProvider)
          .register(() => teardown.future);

      final switching = notifier.switchCommunity(ws2.id);
      await Future<void>.delayed(Duration.zero);

      expect(await communityStorage.loadActiveId(), ws1.id);
      teardown.complete();
      await switching;
      expect(await communityStorage.loadActiveId(), ws2.id);
    });

    test('overlapping transitions share one callback run', () async {
      container = createContainer();
      final coordinator = container.read(communityTransitionProvider);
      final teardown = Completer<void>();
      var calls = 0;
      coordinator.register(() {
        calls++;
        return teardown.future;
      });

      var firstCompleted = false;
      var secondCompleted = false;
      final first = coordinator.run().then((_) => firstCompleted = true);
      final second = coordinator.run().then((_) => secondCompleted = true);
      await Future<void>.delayed(Duration.zero);

      expect(calls, 1);
      expect(firstCompleted, isFalse);
      expect(secondCompleted, isFalse);
      teardown.complete();
      await Future.wait([first, second]);
      expect(firstCompleted, isTrue);
      expect(secondCompleted, isTrue);
    });

    test('a later transition starts a new callback run', () async {
      container = createContainer();
      final coordinator = container.read(communityTransitionProvider);
      var calls = 0;
      coordinator.register(() async => calls++);

      await coordinator.run();
      await coordinator.run();

      expect(calls, 2);
    });

    test(
      'transition callback failure does not block the active ID update',
      () async {
        final ws1 = Community.create(
          name: 'One',
          relayUrl: 'https://one.example.com',
        );
        final ws2 = Community.create(
          name: 'Two',
          relayUrl: 'https://two.example.com',
        );
        await communityStorage.save(ws1);
        await communityStorage.save(ws2);
        container = createContainer();
        await container.read(communityListProvider.future);
        final notifier = container.read(communityListProvider.notifier);
        await notifier.switchCommunity(ws1.id);
        container.read(communityTransitionProvider).register(() async {
          throw StateError('native cleanup failed');
        });

        await notifier.switchCommunity(ws2.id);

        expect(await communityStorage.loadActiveId(), ws2.id);
      },
    );

    test('switchCommunity updates active ID', () async {
      final ws1 = Community.create(
        name: 'One',
        relayUrl: 'https://one.example.com',
      );
      final ws2 = Community.create(
        name: 'Two',
        relayUrl: 'https://two.example.com',
      );

      await communityStorage.save(ws1);
      await communityStorage.save(ws2);
      container = createContainer();
      await container.read(communityListProvider.future);
      final notifier = container.read(communityListProvider.notifier);
      await notifier.switchCommunity(ws2.id);

      final activeId = await communityStorage.loadActiveId();
      expect(activeId, ws2.id);
    });
  });

  test('switching and removing name the new member at once', () async {
    final reporter = _MemberRecorder();
    final one = nostr.Keys.generate();
    final two = nostr.Keys.generate();
    final ws1 = Community.create(
      name: 'One',
      relayUrl: 'https://one.example.com',
      nsec: one.nsec,
    );
    final ws2 = Community.create(
      name: 'Two',
      relayUrl: 'https://two.example.com',
      nsec: two.nsec,
    );
    await communityStorage.save(ws1);
    await communityStorage.save(ws2);
    container = createContainer(
      overrides: [diagnosticReporterProvider.overrideWithValue(reporter)],
    );
    await container.read(communityListProvider.future);
    final notifier = container.read(communityListProvider.notifier);

    await notifier.switchCommunity(ws1.id);
    expect(reporter.member, one.public);
    await notifier.switchCommunity(ws2.id);
    expect(reporter.member, two.public);
    await notifier.removeCommunity(ws2.id);
    expect(reporter.member, one.public);
    await notifier.removeCommunity(ws1.id);
    expect(reporter.member, isNull);
  });

  test('switching names the member from storage, not a stale list', () async {
    final reporter = _MemberRecorder();
    container = createContainer(
      overrides: [diagnosticReporterProvider.overrideWithValue(reporter)],
    );
    await container.read(communityListProvider.future);
    final keys = nostr.Keys.generate();
    final saved = Community.create(
      name: 'Saved',
      relayUrl: 'https://saved.example.com',
      nsec: keys.nsec,
    );
    // Saved, but not yet in the provider's list, as while it reloads after
    // sign-in.
    await communityStorage.save(saved);

    await container
        .read(communityListProvider.notifier)
        .switchCommunity(saved.id);

    expect(reporter.member, keys.public);
  });

  test('a switch whose read fails changes nothing and can retry', () async {
    final storage = _FailingLoadStorage(fakeSecure);
    communityStorage = storage;
    final a = Community.create(
      name: 'A',
      relayUrl: 'https://a.example.com',
      nsec: nostr.Keys.generate().nsec,
    );
    final b = Community.create(
      name: 'B',
      relayUrl: 'https://b.example.com',
      nsec: nostr.Keys.generate().nsec,
    );
    await communityStorage.save(a);
    await communityStorage.save(b);
    container = createContainer();
    final notifier = container.read(communityListProvider.notifier);
    await container.read(communityListProvider.future);
    await notifier.switchCommunity(a.id);

    storage.failNextLoad = true;
    await expectLater(notifier.switchCommunity(b.id), throwsStateError);
    expect(await storage.loadActiveId(), a.id);

    await notifier.switchCommunity(b.id);
    expect(await storage.loadActiveId(), b.id);
    expect((await container.read(activeCommunityProvider.future))?.id, b.id);
  });

  group('activeCommunityProvider', () {
    test('returns null when no communities', () async {
      container = createContainer();
      final active = await container.read(activeCommunityProvider.future);
      expect(active, isNull);
    });

    test('returns community matching active ID', () async {
      final ws = Community.create(
        name: 'Test',
        relayUrl: 'https://test.example.com',
      );
      await communityStorage.save(ws);
      container = createContainer();
      await container.read(communityListProvider.future);
      final notifier = container.read(communityListProvider.notifier);
      await notifier.switchCommunity(ws.id);

      final active = await container.read(activeCommunityProvider.future);
      expect(active, isNotNull);
      expect(active!.id, ws.id);
      expect(active.name, 'Test');
    });

    test('falls back without persisting when active ID is missing', () async {
      final community = Community.create(
        name: 'Fallback',
        relayUrl: 'https://test.example.com',
      );
      await communityStorage.save(community);
      container = createContainer();

      final active = await container.read(activeCommunityProvider.future);
      expect(active?.id, community.id);
      expect(await communityStorage.loadActiveId(), isNull);
    });

    test('falls back to first community if active ID is invalid', () async {
      final ws = Community.create(
        name: 'Fallback',
        relayUrl: 'https://test.example.com',
      );
      await communityStorage.save(ws);
      container = createContainer();
      await container.read(communityListProvider.future);

      // Set an invalid active ID.
      await communityStorage.saveActiveId('nonexistent-id');

      // Re-read — should fall back.
      container.invalidate(activeCommunityProvider);
      final active = await container.read(activeCommunityProvider.future);
      expect(active, isNotNull);
      expect(active!.id, ws.id);
      expect(await communityStorage.loadActiveId(), 'nonexistent-id');
    });
  });
}

class _SnapshotStore extends InMemorySharedPreferencesStore {
  _SnapshotStore() : super.empty();
  String? failure;
  int removeAttempts = 0;
  Completer<void>? removeGate;
  @override
  Future<bool> remove(String key) async {
    removeAttempts++;
    if (failure == 'false') return false;
    if (failure == 'throw') throw StateError('disk failure');
    final removed = super.remove(key);
    await removeGate?.future;
    return removed;
  }
}

const _savedPage = (channelId: 'saved-channel', rootId: '');

NostrEvent _savedMessage(String secret) => NostrEvent.fromJson(
  signNostrEvent(
    kind: EventKind.streamMessage,
    content: 'saved message',
    secretKey: secret,
    createdAt: 1700000000,
    tags: [
      ['h', _savedPage.channelId],
    ],
  ).toMap(),
);

Future<void> _saveMessage(
  SavedMessageStore store,
  SavedMessageScope scope,
  String secret,
) => store.save(store.capture(scope, _savedPage), [
  _savedMessage(secret),
], relayAuthor: null);

/// A fresh ticket reads the database itself, not a retired epoch.
Future<List<NostrEvent>?> _readSaved(
  SavedMessageStore store,
  SavedMessageScope scope,
) => store.read(store.capture(scope, _savedPage));

class _MemberRecorder extends NoopDiagnosticReporter {
  @override
  String? member = 'unset';

  @override
  void setMember(String? pubkey) => member = pubkey;
}

class _FailingLoadStorage extends CommunityStorage {
  _FailingLoadStorage(FlutterSecureStorage secure) : super(secure: secure);

  bool failNextLoad = false;

  @override
  Future<List<Community>> loadAll() {
    if (failNextLoad) {
      failNextLoad = false;
      throw StateError('keychain read failed');
    }
    return super.loadAll();
  }
}

class _MemoryStore implements AndroidPushStateStore {
  String? value;

  @override
  Future<String?> read() async => value;

  @override
  Future<void> write(String? next) async => value = next;
}

Map<String, dynamic> _descriptorJson(String origin) => {
  'supported_extensions': ['nip-er', 'buzz-push-v1'],
  'push': {
    'origin': origin,
    'keys': [
      {
        'id': 'relay-v1',
        'pubkey': nostr.Keys('2' * 64).public,
        'current': true,
      },
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

class _FailingSaveStorage extends CommunityStorage {
  _FailingSaveStorage(FlutterSecureStorage secure) : super(secure: secure);

  bool failNextSave = false;

  @override
  Future<void> save(Community community) {
    if (failNextSave) {
      failNextSave = false;
      throw StateError('keychain write failed');
    }
    return super.save(community);
  }
}
