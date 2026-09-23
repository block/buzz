import 'dart:async';
import 'dart:isolate';

import 'package:buzz/features/channels/channels_provider.dart';
import 'package:buzz/shared/community/community.dart';
import 'package:buzz/shared/community/community_provider.dart';
import 'package:buzz/shared/profile/user_cache_provider.dart';
import 'package:buzz/shared/profile/profile_event_parser.dart';
import 'package:buzz/shared/push/push_presentation_cache.dart';
import 'package:buzz/shared/push/push_presentation_export_recovery.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:fake_async/fake_async.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;

const _bridge = MethodChannel('buzz/push');

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  final messenger =
      TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger;
  setUp(() {
    debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
    pushPresentationExportError.value = null;
  });
  tearDown(() {
    debugDefaultTargetPlatformOverride = null;
    pushPresentationExportError.value = null;
    messenger.setMockMethodCallHandler(_bridge, null);
  });

  for (final profiles in [true, false]) {
    final section = profiles ? 'profiles' : 'channels';
    test(
      '$section producer recovers its saturated detached snapshot',
      () async {
        final blocked = Completer<void>();
        final entered = Completer<void>();
        final delivered = Completer<Map<dynamic, dynamic>>();
        messenger.setMockMethodCallHandler(_bridge, (call) async {
          final args = call.arguments as Map<dynamic, dynamic>;
          if (args['communityId'] == 'blocker' && !entered.isCompleted) {
            entered.complete();
            await blocked.future;
          }
          if (args['communityId'] == 'original' && !delivered.isCompleted) {
            delivered.complete(args);
          }
          return null;
        });
        final filler = _signed(0);
        final queued = [
          cacheBuzzPushProfileEvents('blocker', [filler]),
        ];
        await entered.future;
        for (var i = 1; i < 8; i++) {
          queued.add(cacheBuzzPushProfileEvents('queued-$i', [filler]));
        }
        final event = _signed(profiles ? 0 : 39000);
        final membership = _signed(39002);
        var communityID = 'original';
        final container = _container(
          _Session(event, membership),
          communityID: () => communityID,
        );
        addTearDown(container.dispose);
        Future<bool>? preload;
        try {
          await container.read(activeCommunityProvider.future);
          if (profiles) {
            final ready = Completer<void>();
            final subscription = container.listen(userCacheProvider, (_, next) {
              if (next.containsKey(event.pubkey) && !ready.isCompleted) {
                ready.complete();
              }
            });
            addTearDown(subscription.close);
            preload = container.read(userCacheProvider.notifier).preload([
              event.pubkey,
            ]);
            await ready.future.timeout(const Duration(seconds: 5));
          } else {
            await container.read(channelsProvider.future);
          }
          expect(delivered.isCompleted, isFalse);
          communityID = 'replacement';
          container.invalidate(activeCommunityProvider);
          await container.read(activeCommunityProvider.future);
        } finally {
          blocked.complete();
          await Future.wait(queued);
          if (preload != null) expect(await preload, isTrue);
        }
        final payload = await delivered.future.timeout(
          const Duration(seconds: 5),
        );
        expect(payload['section'], section);
        expect(payload['communityId'], 'original');
        expect(payload[profiles ? 'events' : 'metadataEvents'], [
          event.toJson(),
        ]);
        if (!profiles) {
          expect(payload['membershipEvents'], [membership.toJson()]);
        }
        expect(pushPresentationExportError.value, isNull);
      },
    );

    test(
      '$section producer preserves distinct input beyond eight arrivals',
      () async {
        final entered = Completer<void>();
        final release = Completer<void>();
        final payloads = <Map<dynamic, dynamic>>[];
        final delivered = Completer<void>();
        final expected = <String>{};
        messenger.setMockMethodCallHandler(_bridge, (call) async {
          final args = call.arguments as Map<dynamic, dynamic>;
          if (args['communityId'] == 'blocker' && !entered.isCompleted) {
            entered.complete();
            await release.future;
          }
          if (args['communityId'] == 'original') {
            payloads.add(args);
            final ids = <String>{
              for (final payload in payloads)
                for (final event
                    in payload[profiles ? 'events' : 'metadataEvents'] as List)
                  event['id'] as String,
            };
            if (expected.length == 13 &&
                ids.containsAll(expected) &&
                !delivered.isCompleted) {
              delivered.complete();
            }
          }
          return null;
        });
        final queued = [
          cacheBuzzPushProfileEvents('blocker', [_signed(0)]),
        ];
        await entered.future;
        for (var i = 1; i < 8; i++) {
          queued.add(cacheBuzzPushProfileEvents('queued-$i', [_signed(0)]));
        }
        final first = _signed(profiles ? 0 : 39000);
        expected.add(first.id);
        final session = _Session(first, _signed(39002));
        final container = _container(session);
        addTearDown(container.dispose);
        final loads = <Future<bool>>[];
        List<String>? visibleBefore;
        var subscriptionsBefore = 0;
        try {
          await container.read(activeCommunityProvider.future);
          if (profiles) {
            final ready = Completer<void>();
            final subscription = container.listen(userCacheProvider, (_, next) {
              if (next.containsKey(first.pubkey) && !ready.isCompleted) {
                ready.complete();
              }
            });
            addTearDown(subscription.close);
            loads.add(
              container.read(userCacheProvider.notifier).preload([
                first.pubkey,
              ]),
            );
            await ready.future.timeout(const Duration(seconds: 5));
            expect(await loads.single, isTrue);
          } else {
            await container.read(channelsProvider.future);
          }
          for (var i = 2; i <= 13; i++) {
            final event = _signed(
              profiles ? 0 : 39000,
              key: i,
              channel: 'channel-$i',
            );
            session.extra.add(event);
            expected.add(event.id);
            if (profiles) {
              loads.add(
                container.read(userCacheProvider.notifier).preload([
                  event.pubkey,
                ]),
              );
            } else {
              session.extra.add(_signed(39002, channel: 'channel-$i'));
              await container.read(channelsProvider.notifier).refresh();
            }
          }
          // Include a newer valid event and an invalid newest candidate for an
          // entity in a pending partial batch. Selection must keep the valid one.
          final newer = _signed(
            profiles ? 0 : 39000,
            key: 2,
            channel: 'channel-2',
            createdAt: 200,
          );
          expected.remove(
            _signed(profiles ? 0 : 39000, key: 2, channel: 'channel-2').id,
          );
          expected.add(newer.id);
          session.extra.addAll([
            newer,
            NostrEvent(
              id: 'invalid-newest',
              pubkey: newer.pubkey,
              createdAt: 300,
              kind: newer.kind,
              tags: newer.tags,
              content: 'tampered',
              sig: newer.sig,
            ),
          ]);
          if (profiles) {
            // Let the 50 ms fetch-coalescing timer fire if backpressure is
            // removed. This is scheduling coverage, not a latency threshold.
            await Future<void>.delayed(const Duration(milliseconds: 60));
            expect(
              session.profileFetches,
              1,
              reason: 'pending keys must remain upstream while export waits',
            );
          } else {
            for (var turn = 0; turn < 20; turn++) {
              await Future<void>.delayed(Duration.zero);
            }
            visibleBefore = container
                .read(channelsProvider)
                .requireValue
                .map((channel) => channel.id)
                .toList();
            subscriptionsBefore = session.subscriptions;
          }
        } finally {
          release.complete();
          await Future.wait(queued);
        }
        await Future.wait(loads);
        await delivered.future.timeout(const Duration(seconds: 5));
        expect(pushPresentationExportError.value, isNull);
        if (!profiles) {
          expect(
            container
                .read(channelsProvider)
                .requireValue
                .map((channel) => channel.id),
            visibleBefore,
          );
          expect(session.subscriptions, subscriptionsBefore);
          expect(
            session.directoryLoads,
            0,
            reason: 'notification recovery must not fetch the open directory',
          );
          expect(
            payloads.last['membershipEvents'],
            unorderedEquals([
              session.membership.toJson(),
              for (final event in session.extra)
                if (event.kind == 39002) event.toJson(),
            ]),
          );
        }
      },
    );

    test(
      '$section producer handles detached worker submission failure',
      () async {
        final port = ReceivePort();
        addTearDown(port.close);
        final event = _Unsendable(_signed(profiles ? 0 : 39000), port);
        final failure = Completer<void>();
        void onError() {
          if (pushPresentationExportError.value != null &&
              !failure.isCompleted) {
            failure.complete();
          }
        }

        pushPresentationExportError.addListener(onError);
        addTearDown(() => pushPresentationExportError.removeListener(onError));
        var nativeCalls = 0;
        messenger.setMockMethodCallHandler(_bridge, (_) async {
          nativeCalls++;
          return null;
        });
        final container = _container(
          _Session(event, _signed(39002)),
          bypassProfileWorker: true,
        );
        addTearDown(container.dispose);
        await container.read(activeCommunityProvider.future);
        if (profiles) {
          await container.read(userCacheProvider.notifier).preload([
            event.pubkey,
          ]);
        } else {
          await container.read(channelsProvider.future);
        }
        await failure.future.timeout(const Duration(seconds: 5));
        expect(nativeCalls, 0);
        final terminalError = pushPresentationExportError.value;
        await cacheBuzzPushProfileEvents('unrelated-success', [_signed(0)]);
        expect(nativeCalls, 1);
        expect(pushPresentationExportError.value, terminalError);
      },
    );
  }

  for (final mode in ['reentrant', 'retired', 'failure']) {
    test('channel dirty refetch handles $mode work', () async {
      final nativeEntered = Completer<void>();
      final releaseNative = Completer<void>();
      final refetchEntered = Completer<void>();
      final releaseRefetch = Completer<void>();
      final complete = Completer<void>();
      final payloads = <Map<dynamic, dynamic>>[];
      messenger.setMockMethodCallHandler(_bridge, (call) async {
        final args = call.arguments as Map<dynamic, dynamic>;
        payloads.add(args);
        if (payloads.length == 1) {
          nativeEntered.complete();
          await releaseNative.future;
        } else if ((mode == 'reentrant' && payloads.length == 3) ||
            (mode == 'retired' && args['communityId'] == 'replacement')) {
          if (!complete.isCompleted) complete.complete();
        }
        return null;
      });
      void errorChanged() {
        if (mode == 'failure' &&
            pushPresentationExportError.value != null &&
            !complete.isCompleted) {
          complete.complete();
        }
      }

      pushPresentationExportError.addListener(errorChanged);
      addTearDown(
        () => pushPresentationExportError.removeListener(errorChanged),
      );
      final session = _Session(_signed(39000), _signed(39002));
      var community = 'original';
      final container = _container(session, communityID: () => community);
      addTearDown(container.dispose);
      await container.read(activeCommunityProvider.future);
      await container.read(channelsProvider.future);
      await nativeEntered.future;
      await container.read(channelsProvider.notifier).refresh();
      session.beforeMetadata = () async {
        refetchEntered.complete();
        await releaseRefetch.future;
        if (mode == 'failure') throw StateError('synthetic refetch failure');
      };
      releaseNative.complete();
      await refetchEntered.future.timeout(const Duration(seconds: 5));
      try {
        if (mode == 'reentrant') {
          session.extra.addAll([
            _signed(39000, channel: 'later'),
            _signed(39002, channel: 'later'),
          ]);
          await container.read(channelsProvider.notifier).refresh();
        } else if (mode == 'retired') {
          community = 'replacement';
          session.event = _signed(39000, channel: 'replacement-channel');
          session.membership = _signed(39002, channel: 'replacement-channel');
          container.invalidate(activeCommunityProvider);
          await container.read(activeCommunityProvider.future);
          container
              .read(relayConfigProvider.notifier)
              .update(baseUrl: 'https://replacement.invalid');
          await container.read(channelsProvider.future);
          await container.read(channelsProvider.notifier).refresh();
        }
      } finally {
        releaseRefetch.complete();
      }
      await complete.future.timeout(const Duration(seconds: 5));
      for (var turn = 0; turn < 20; turn++) {
        await Future<void>.delayed(Duration.zero);
      }
      if (mode == 'failure') {
        expect(session.metadataLoads, 3);
        expect(session.directoryLoads, 0);
        expect(payloads, hasLength(1));
        expect(
          pushPresentationExportError.value,
          contains('synthetic refetch failure'),
        );
      } else if (mode == 'retired') {
        expect(
          payloads.where((payload) => payload['communityId'] == 'original'),
          hasLength(1),
        );
        expect(payloads.last['metadataEvents'], [session.event.toJson()]);
        expect(payloads.last['membershipEvents'], [
          session.membership.toJson(),
        ]);
        expect(pushPresentationExportError.value, isNull);
      } else {
        expect(session.metadataLoads, 5);
        expect(session.directoryLoads, 0);
        expect(payloads.last['metadataEvents'], hasLength(2));
        expect(payloads.last['membershipEvents'], hasLength(2));
        expect(pushPresentationExportError.value, isNull);
      }
    });
  }

  for (final failedOperation in ['initial export', 'dirty refetch']) {
    test(
      'pending channel input drains after exhausted $failedOperation',
      () async {
        final blockerEntered = Completer<void>();
        final releaseBlocker = Completer<void>();
        final initialEntered = Completer<void>();
        final releaseInitial = Completer<void>();
        final refetchEntered = Completer<void>();
        final releaseRefetch = Completer<void>();
        final terminal = Completer<void>();
        late _Session session;
        List<int>? readsBeforeAdmissionRetries;
        List<int>? readsAtTerminal;
        final latestDelivered = Completer<Map<dynamic, dynamic>>();
        final latest = _signed(39000, channel: 'latest');
        final latestMembership = _signed(39002, channel: 'latest');
        messenger.setMockMethodCallHandler(_bridge, (call) async {
          final args = call.arguments as Map<dynamic, dynamic>;
          if (args['communityId'] == 'blocker' && !blockerEntered.isCompleted) {
            blockerEntered.complete();
            await releaseBlocker.future;
          }
          if (args['communityId'] == 'original') {
            if (failedOperation == 'dirty refetch' &&
                !initialEntered.isCompleted) {
              initialEntered.complete();
              await releaseInitial.future;
            }
            if ((args['metadataEvents'] as List).any(
                  (event) => event['id'] == latest.id,
                ) &&
                !latestDelivered.isCompleted) {
              latestDelivered.complete(args);
            }
          }
          return null;
        });
        void onFailure() {
          if (pushPresentationExportError.value != null &&
              !terminal.isCompleted) {
            readsAtTerminal = session.snapshotReadCounts;
            terminal.complete();
          }
        }

        pushPresentationExportError.addListener(onFailure);
        addTearDown(
          () => pushPresentationExportError.removeListener(onFailure),
        );
        final queued = <Future<void>>[];
        Future<void> saturate() async {
          queued.add(cacheBuzzPushProfileEvents('blocker', [_signed(0)]));
          await blockerEntered.future;
          for (var i = 1; i < 8; i++) {
            queued.add(cacheBuzzPushProfileEvents('queued-$i', [_signed(0)]));
          }
        }

        session = _Session(_signed(39000), _signed(39002));
        final container = _container(session);
        addTearDown(container.dispose);
        try {
          if (failedOperation == 'initial export') await saturate();
          await container.read(activeCommunityProvider.future);
          await container.read(channelsProvider.future);
          if (failedOperation == 'dirty refetch') {
            await initialEntered.future;
            await container.read(channelsProvider.notifier).refresh();
            session.beforeMetadata = () async {
              refetchEntered.complete();
              await releaseRefetch.future;
            };
            releaseInitial.complete();
            await refetchEntered.future.timeout(const Duration(seconds: 5));
            await saturate();
          }
          session.extra.addAll([latest, latestMembership]);
          await container.read(channelsProvider.notifier).refresh();
          readsBeforeAdmissionRetries = session.snapshotReadCounts;
          if (!releaseRefetch.isCompleted) releaseRefetch.complete();
          await terminal.future.timeout(const Duration(seconds: 12));
          expect(pushPresentationExportError.value, contains('queue is full'));
          if (failedOperation == 'dirty refetch') {
            // Membership pages and metadata were fetched before the paused
            // metadata response. Only the remaining member snapshot is read;
            // all five admission retries must reuse those same raw events.
            expect(
              [
                for (var i = 0; i < 3; i++)
                  readsAtTerminal![i] - readsBeforeAdmissionRetries[i],
              ],
              [0, 0, 1],
            );
          }
          // Capacity returns only after the predecessor has exhausted every retry.
          releaseBlocker.complete();
          await Future.wait(queued);
          final payload = await latestDelivered.future.timeout(
            const Duration(seconds: 5),
          );
          expect(payload['communityId'], 'original');
          expect(payload['metadataEvents'], contains(equals(latest.toJson())));
          expect(
            payload['membershipEvents'],
            contains(equals(latestMembership.toJson())),
          );
          expect(session.directoryLoads, 0);
          expect(pushPresentationExportError.value, contains('queue is full'));
        } finally {
          if (!releaseBlocker.isCompleted) releaseBlocker.complete();
          if (!releaseInitial.isCompleted) releaseInitial.complete();
          if (!releaseRefetch.isCompleted) releaseRefetch.complete();
          await Future.wait(queued);
        }
      },
    );
  }

  test('bounded retries terminate and a later operation can succeed', () {
    fakeAsync((clock) {
      final recovery = PushPresentationExportRecovery();
      var attempts = 0;
      bool? result;
      recovery
          .export(() async {
            attempts++;
            throw PushPresentationExportQueueFull();
          })
          .then((value) => result = value);
      clock.flushMicrotasks();
      clock.elapse(const Duration(seconds: 8));
      expect(result, isFalse);
      expect(attempts, 6);
      final terminal = pushPresentationExportError.value;
      expect(terminal, isNotNull);
      clock.elapse(const Duration(days: 1));
      expect(attempts, 6);
      bool? recovered;
      recovery.export(() async {}).then((value) => recovered = value);
      clock.flushMicrotasks();
      expect(recovered, isTrue);
      expect(pushPresentationExportError.value, terminal);
    });
  });
}

ProviderContainer _container(
  _Session session, {
  String Function()? communityID,
  bool bypassProfileWorker = false,
}) => ProviderContainer(
  retry: (_, _) => null,
  overrides: [
    if (bypassProfileWorker)
      profileEventBatchParserProvider.overrideWithValue(
        (events) async => events.map(parseProfileEvent).toList(),
      ),
    relaySessionProvider.overrideWith(() => session),
    appLifecycleProvider.overrideWith(_Lifecycle.new),
    myPubkeyProvider.overrideWith((ref) => 'me'),
    activeCommunityProvider.overrideWith(
      (ref) async => Community(
        id: communityID?.call() ?? 'original',
        name: 'Synthetic',
        relayUrl: 'https://example.invalid',
        addedAt: DateTime(2026),
      ),
    ),
  ],
);

class _Session extends RelaySessionNotifier {
  _Session(this.event, this.membership);
  NostrEvent event;
  NostrEvent membership;
  final extra = <NostrEvent>[];
  int profileFetches = 0;
  int subscriptions = 0;
  int directoryLoads = 0;
  int metadataLoads = 0;
  int membershipPages = 0;
  int memberSnapshotLoads = 0;
  List<int> get snapshotReadCounts => [
    membershipPages,
    metadataLoads,
    memberSnapshotLoads,
  ];
  Future<void> Function()? beforeMetadata;
  @override
  SessionState build() => const SessionState(status: SessionStatus.connected);
  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    if (filter.kinds.contains(0)) profileFetches++;
    if (filter.kinds.contains(39002) && filter.tags['#d'] != null) {
      memberSnapshotLoads++;
    }
    if (filter.kinds.contains(39000)) {
      metadataLoads++;
      final hook = beforeMetadata;
      beforeMetadata = null;
      if (hook != null) await hook();
    }
    return [
      for (final candidate in [event, membership, ...extra])
        if (filter.kinds.contains(candidate.kind) &&
            (filter.authors == null ||
                filter.authors!.contains(candidate.pubkey)))
          candidate,
    ];
  }

  @override
  Future<List<NostrEvent>> queryRelay(
    List<NostrFilter> filters, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    if (filters.any(
      (filter) => filter.kinds.contains(39002) && filter.tags['#p'] != null,
    )) {
      membershipPages++;
    }
    if (filters.any(
      (filter) =>
          filter.kinds.contains(39000) &&
          filter.tags['#d'] == null &&
          filter.extensions['before_id'] == null,
    )) {
      directoryLoads++;
    }
    return [
      for (final filter in filters)
        ...await fetchHistory(filter, timeout: timeout),
    ];
  }

  @override
  Future<void Function()> subscribe(
    NostrFilter filter,
    void Function(NostrEvent) onEvent, {
    void Function(String)? onClosed,
  }) async {
    subscriptions++;
    return () {};
  }
}

class _Lifecycle extends AppLifecycleNotifier {
  @override
  AppLifecycleState build() => AppLifecycleState.resumed;
}

NostrEvent _signed(
  int kind, {
  int key = 1,
  String channel = 'channel',
  int createdAt = 100,
}) => NostrEvent.fromJson(
  nostr.Event.from(
    kind: kind,
    createdAt: createdAt,
    content: kind == 0 ? '{"name":"Synthetic"}' : '',
    tags: kind == 0
        ? []
        : [
            ['d', channel],
            ['name', 'Synthetic'],
            if (kind == 39002) ['p', 'me'],
          ],
    secretKey: key.toRadixString(16).padLeft(64, '0'),
  ).toMap(),
);

class _Unsendable extends NostrEvent {
  final ReceivePort port;
  _Unsendable(NostrEvent event, this.port)
    : super(
        id: event.id,
        pubkey: event.pubkey,
        createdAt: event.createdAt,
        kind: event.kind,
        tags: event.tags,
        content: event.content,
        sig: event.sig,
      );
}
