import 'dart:async';

import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channels_provider.dart';
import 'package:buzz/features/channels/channel_unread.dart';
import 'package:buzz/shared/read_state/read_state_provider.dart';
import 'package:buzz/features/channels/thread_follows/thread_follows_provider.dart';
import 'package:buzz/features/channels/channel_mutes/channel_mutes_provider.dart';
import 'package:buzz/features/channels/channel_mutes/channel_mutes_storage.dart';
import 'package:buzz/features/channels/unread_badge/unread_badge_provider.dart';
import 'package:buzz/shared/channels/channel_list_snapshot.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/theme_provider.dart';
import 'package:fake_async/fake_async.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';

part 'channel_snapshot_restored_cases.dart';

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  _restoredLaunchCases();
  test(
    'following a thread before restart recovers its older replies',
    () async {
      SharedPreferences.setMockInitialValues({});
      final prefs = await SharedPreferences.getInstance();
      final root = 'a' * 64;
      final reply = NostrEvent(
        id: 'reply',
        pubkey: 'alice',
        createdAt: DateTime.now().millisecondsSinceEpoch ~/ 1000 - 7200,
        kind: EventKind.streamMessage,
        tags: [
          ['h', 'alpha'],
          ['e', root, '', 'root'],
          ['e', root, '', 'reply'],
        ],
        content: 'reply',
        sig: 'sig',
      );
      final first = _container(
        prefs,
        _SnapshotSession(['alpha'])..unreadMessages.add(reply),
      );
      await first.read(channelsProvider.future);
      await _settle();
      expect(
        first
            .read(channelsProvider.notifier)
            .observedThreadReplies(first.read(channelsProvider).requireValue),
        isEmpty,
      );
      first.read(threadFollowsProvider.notifier).followThread(root);
      await _settle();
      first.dispose();
      final second = _container(
        prefs,
        _SnapshotSession(['alpha'])..unreadMessages.add(reply),
      );
      addTearDown(second.dispose);
      await second.read(channelsProvider.future);
      await _settle();
      expect(
        second.read(threadFollowsProvider).followedRootIds,
        contains(root),
      );
      expect(
        second
            .read(channelsProvider.notifier)
            .observedThreadReplies(second.read(channelsProvider).requireValue)
            .map((e) => e.id),
        contains('reply'),
      );
    },
  );

  test(
    'cold start with a changed mute removes ordinary unread evidence',
    () async {
      SharedPreferences.setMockInitialValues({});
      final prefs = await SharedPreferences.getInstance();
      final message = NostrEvent(
        id: 'ordinary',
        pubkey: 'alice',
        createdAt: DateTime.now().millisecondsSinceEpoch ~/ 1000 - 7200,
        kind: EventKind.streamMessage,
        tags: [
          ['h', 'alpha'],
        ],
        content: 'ordinary',
        sig: 'sig',
      );
      final first = _container(
        prefs,
        _SnapshotSession(['alpha'])..unreadMessages.add(message),
      );
      await first.read(channelsProvider.future);
      await _settle();
      expect(first.read(unreadBadgeProvider).generalUnreadCount, 1);
      first.dispose();
      final session = _SnapshotSession(['alpha'])
        ..unreadMessages.add(message)
        ..unreadGate = Completer();
      final second = _container(prefs, session, muted: true);
      addTearDown(second.dispose);
      await second.read(channelsProvider.future);
      await _settle();
      // A mention arrives live while the full window is out, and the window
      // was read before it. It is told apart by when it arrived, not by its
      // createdAt, which is older than the window's start here.
      final mention = NostrEvent(
        id: 'live-mention',
        pubkey: 'alice',
        createdAt: message.createdAt + 1,
        kind: EventKind.streamMessage,
        tags: [
          ['h', 'alpha'],
          ['p', 'me'],
        ],
        content: 'mention',
        sig: 'sig',
      );
      expect(session.unreadFilters, isNotEmpty);
      session.emit(mention);
      session.unreadGate!.complete();
      await _settle();
      expect(
        session.unreadFilters.where((f) => f.ids == null).map((f) => f.since),
        everyElement(0),
      );
      expect(second.read(unreadBadgeProvider).generalUnreadCount, 0);
      expect(second.read(unreadBadgeProvider).highPriorityCount, 1);
      expect(
        second
            .read(channelsProvider.notifier)
            .observedUnreadEventsByChannel['alpha']
            ?.keys,
        ['live-mention'],
      );
      second.dispose();

      // Saved after the mute: the next cold start keeps the mention and
      // drops nothing it should show, and asks only for what is new.
      final thirdSession = _SnapshotSession(['alpha'])
        ..unreadMessages.addAll([message, mention]);
      final third = _container(prefs, thirdSession, muted: true);
      addTearDown(third.dispose);
      await third.read(channelsProvider.future);
      await _settle();
      expect(
        thirdSession.unreadFilters
            .where((f) => f.ids == null)
            .map((f) => f.since),
        everyElement(greaterThan(0)),
      );
      expect(third.read(unreadBadgeProvider).generalUnreadCount, 0);
      expect(third.read(unreadBadgeProvider).highPriorityCount, 1);
    },
  );

  test('following threads in session recovers their older replies, then and '
      'after a restart', () async {
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();
    final root = 'b' * 64;
    final reply = NostrEvent(
      id: 'in-session-reply',
      pubkey: 'alice',
      createdAt: DateTime.now().millisecondsSinceEpoch ~/ 1000 - 7200,
      kind: EventKind.streamMessage,
      tags: [
        ['h', 'alpha'],
        ['e', root, '', 'root'],
        ['e', root, '', 'reply'],
      ],
      content: 'reply',
      sig: 'sig',
    );
    Iterable<String> activity(ProviderContainer container) => container
        .read(channelsProvider.notifier)
        .observedThreadReplies(container.read(channelsProvider).requireValue)
        .map((e) => e.id);
    final firstSession = _SnapshotSession(['alpha', 'beta'])
      ..unreadMessages.add(reply);
    final first = _container(prefs, firstSession);
    addTearDown(first.dispose);
    await first.read(channelsProvider.future);
    await _settle();
    expect(activity(first), isEmpty);

    // As the Follow thread action does.
    first.read(threadFollowsProvider.notifier).followThread(root);
    first.read(channelsProvider.notifier).noteThreadFollowChanged('alpha');
    // A second follow elsewhere before the catch-up keeps alpha's reset.
    first.read(threadFollowsProvider.notifier).followThread('c' * 64);
    first.read(channelsProvider.notifier).noteThreadFollowChanged('beta');
    firstSession.unreadFilters.clear();
    await first.read(channelsProvider.notifier).refresh();
    await first.read(channelsProvider.notifier).waitForUnreadCatchUp();
    expect(
      firstSession.unreadFilters
          .where((f) => f.ids == null)
          .map((f) => f.since),
      everyElement(0),
    );
    expect(activity(first), ['in-session-reply']);
    first.dispose();

    final secondSession = _SnapshotSession(['alpha', 'beta'])
      ..unreadMessages.add(reply);
    final second = _container(prefs, secondSession);
    addTearDown(second.dispose);
    await second.read(channelsProvider.future);
    await _settle();
    expect(
      secondSession.unreadFilters
          .where((f) => f.ids == null)
          .map((f) => f.since),
      everyElement(greaterThan(0)),
    );
    expect(activity(second), ['in-session-reply']);
  });
  test(
    'a cold start restores unread state and asks only for newer messages',
    () async {
      SharedPreferences.setMockInitialValues({});
      final prefs = await SharedPreferences.getInstance();
      int now() => DateTime.now().millisecondsSinceEpoch ~/ 1000;
      NostrEvent mention(String id, int createdAt, {String? root}) =>
          NostrEvent(
            id: id,
            pubkey: 'alice',
            createdAt: createdAt,
            kind: EventKind.streamMessage,
            tags: [
              ['h', 'alpha'],
              ['p', 'me'],
              if (root != null) ...[
                ['e', root, '', 'root'],
                ['e', root, '', 'reply'],
              ],
            ],
            content: 'hi @me',
            sig: 'sig',
          );
      final old = mention('old', now() - 7200);
      final reply = mention('reply', now() - 7200, root: 'a' * 64);
      final gone = mention('gone', now() - 7200, root: 'a' * 64);
      List<int?> unreadSince(_SnapshotSession session) => [
        for (final filter in session.unreadFilters)
          if (filter.kinds.contains(EventKind.streamMessage) &&
              filter.ids == null)
            filter.since,
      ];

      final first = _SnapshotSession(['alpha'])
        ..unreadMessages.addAll([old, reply, gone]);
      final firstContainer = _container(prefs, first);
      addTearDown(firstContainer.dispose);
      final firstSentFrom = now();
      await firstContainer.read(channelsProvider.future);
      await _settle();
      final firstSentTo = now();
      expect(unreadSince(first), [0]);
      expect(
        firstContainer
            .read(channelsProvider.notifier)
            .observedUnreadEventsByChannel['alpha']
            ?.keys,
        unorderedEquals(['old', 'reply', 'gone']),
      );
      firstContainer.dispose();

      // The relay no longer has `gone`.
      final second = _SnapshotSession(['alpha'])
        ..unreadMessages.addAll([old, reply, mention('new', now())]);
      final secondContainer = _container(prefs, second);
      addTearDown(secondContainer.dispose);
      await secondContainer.read(channelsProvider.future);
      await _settle();
      expect(
        unreadSince(second).single,
        inInclusiveRange(
          firstSentFrom - relayCrossDeviceSkewSeconds,
          firstSentTo - relayCrossDeviceSkewSeconds,
        ),
      );
      expect(
        secondContainer
            .read(channelsProvider.notifier)
            .observedUnreadEventsByChannel['alpha']
            ?.keys,
        unorderedEquals(['old', 'reply', 'new']),
      );
      // Thread replies were saved without their payload, which Activity
      // shows, and asked for by ID.
      expect([
        for (final filter in second.unreadFilters) ...?filter.ids,
      ], unorderedEquals(['reply', 'gone']));
      final channels = secondContainer.read(channelsProvider).requireValue;
      expect(
        secondContainer
            .read(channelsProvider.notifier)
            .observedThreadReplies(channels)
            .map((event) => event.id),
        ['reply'],
      );
    },
  );

  test('fresh read retries a failed initial list', () async {
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();
    final session = _SnapshotSession(['alpha'])..fail = true;
    final container = _container(prefs, session);
    addTearDown(container.dispose);
    await expectLater(
      container.read(channelsProvider.future),
      throwsStateError,
    );
    session.fail = false;
    expect(
      (await container.read(channelsProvider.notifier).fetchFresh()).single.id,
      'alpha',
    );
    expect(container.read(channelsProvider).requireValue.single.id, 'alpha');
  });

  test(
    'fresh membership reads propagate failures while retaining cached UI',
    () async {
      SharedPreferences.setMockInitialValues({});
      final prefs = await SharedPreferences.getInstance();
      final session = _SnapshotSession(['alpha']);
      final container = _container(prefs, session);
      addTearDown(container.dispose);
      await container.read(channelsProvider.future);
      await _settle();
      session.ids = ['alpha', 'me-chat'];
      final notifier = container.read(channelsProvider.notifier);
      final fresh = await notifier.fetchFresh();
      expect(fresh.map((c) => c.id), ['alpha', 'me-chat']);
      session.fail = true;
      await expectLater(
        notifier.fetchFresh(fetchDirectory: true),
        throwsStateError,
      );
      expect(container.read(channelsProvider).requireValue.map((c) => c.id), [
        'alpha',
        'me-chat',
      ]);
      session.disconnect();
      await expectLater(notifier.fetchFresh(), throwsStateError);
      expect(container.read(channelsProvider).hasError, isFalse);
    },
  );

  test('history deadline keeps full snapshot revalidation pending', () async {
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();
    final storage = ChannelListSnapshotStorage(prefs);
    await storage.write(
      storage.capture('me', 'https://a.example'),
      ChannelListSnapshot(
        channels: [_channel('alpha').toJson()],
        hiddenDmIds: {},
      ),
    );
    fakeAsync((clock) {
      final session = _SnapshotSession(['alpha'])..latestTimeout = true;
      final container = _container(prefs, session);
      container.listen(channelsProvider, (_, _) {});
      clock.flushMicrotasks();
      expect(container.read(channelsProvider).requireValue.single.id, 'alpha');
      expect(session.latestReads, 1);
      session.latestTimeout = false;
      clock.elapse(const Duration(minutes: 1));
      clock.flushMicrotasks();
      expect(session.latestReads, 2);
      clock.elapse(const Duration(minutes: 1));
      clock.flushMicrotasks();
      expect(session.latestReads, 2);
      container.dispose();
      expect(clock.pendingTimers, isEmpty);
    });
  });

  test(
    'reconnect during background refresh queues one non-overlapping retry',
    () async {
      SharedPreferences.setMockInitialValues({});
      final prefs = await SharedPreferences.getInstance();
      fakeAsync((clock) {
        final session = _SnapshotSession(['alpha']);
        final container = _container(prefs, session);
        container.listen(channelsProvider, (_, _) {});
        clock.flushMicrotasks();
        final latestReads = session.latestReads;
        final gate = session.gate = Completer<void>();
        // A live notice marks the list changed, so the next tick reads it.
        session.emit(
          session.event(EventKind.memberAddedNotification, [
            ['p', 'me'],
            ['h', 'alpha'],
          ]),
        );
        clock.elapse(const Duration(minutes: 1));
        clock.flushMicrotasks();
        final parkedRequests = session.membershipRequests;
        session.disconnect();
        session.connect();
        clock.flushMicrotasks();
        expect(session.membershipRequests, parkedRequests);
        session.gate = null;
        gate.complete();
        clock.flushMicrotasks();
        expect(session.membershipRequests, parkedRequests + 1);
        expect(session.latestReads, latestReads);
        container.dispose();
        clock.elapse(const Duration(minutes: 2));
        expect(clock.pendingTimers, isEmpty);
      });
    },
  );

  test('same-scope rebuild keeps live timestamps ahead of disk', () async {
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();
    final session = _SnapshotSession(['alpha']);
    final container = _container(prefs, session);
    addTearDown(container.dispose);
    container.listen(channelsProvider, (_, _) {});
    await container.read(channelsProvider.future);
    await _settle();
    final event = NostrEvent(
      id: 'live',
      pubkey: 'other',
      createdAt: 100,
      kind: 9,
      tags: [
        ['h', 'alpha'],
        ['p', 'me'],
      ],
      content: 'hello',
      sig: 'sig',
    );
    for (final listener in session.listeners.toList()) {
      listener(event);
    }
    final latest = container
        .read(channelsProvider)
        .requireValue
        .single
        .lastMessageAt;
    expect(latest, DateTime.fromMillisecondsSinceEpoch(100000, isUtc: true));
    expect(
      ChannelListSnapshotStorage(
        prefs,
      ).read('me', 'https://a.example')!.channels.single['last_message_at'],
      isNull,
    );
    session.disconnect();
    container.invalidate(channelsProvider);
    expect(
      (await container.read(channelsProvider.future)).single.lastMessageAt,
      latest,
    );
  });

  test(
    'cached discovery survives offline restart and failed refresh, then replaces removed channels',
    () async {
      SharedPreferences.setMockInitialValues({});
      final prefs = await SharedPreferences.getInstance();
      final session = _SnapshotSession(['alpha', 'bravo']);
      var container = _container(prefs, session);
      expect((await container.read(channelsProvider.future)).map((c) => c.id), [
        'alpha',
        'bravo',
      ]);
      await _settle();
      container.dispose();

      fakeAsync((clock) {
        final offline = _SnapshotSession([], connected: false);
        container = _container(prefs, offline);
        container.listen(channelsProvider, (_, _) {});
        clock.flushMicrotasks();
        expect(container.read(channelsProvider).requireValue.map((c) => c.id), [
          'alpha',
          'bravo',
        ]);
        expect(
          container.read(channelsProvider.notifier).hasCachedSnapshot,
          isTrue,
        );
        expect(container.read(channelsProvider.notifier).hiddenDmIds, {
          'hidden-dm',
        });
        expect(offline.membershipRequests, 0);

        offline.fail = true;
        offline.connect();
        clock.flushMicrotasks();
        clock.elapse(const Duration(minutes: 2));
        clock.flushMicrotasks();
        // Retried ever less often: 5s doubling, +/-25%.
        expect(offline.membershipRequests, inInclusiveRange(5, 6));
        expect(offline.listeners, isEmpty);
        // A slow attempt cannot accumulate overlapping retries.
        final gate = Completer<void>();
        offline.gate = gate;
        clock.elapse(const Duration(minutes: 7));
        clock.flushMicrotasks();
        final parkedRequests = offline.membershipRequests;
        clock.elapse(const Duration(minutes: 7));
        clock.flushMicrotasks();
        expect(offline.membershipRequests, parkedRequests);
        gate.complete();
        clock.flushMicrotasks();
        offline.gate = null;
        expect(container.read(channelsProvider).hasError, isFalse);
        expect(container.read(channelsProvider).requireValue, hasLength(2));

        offline.fail = false;
        offline.ids = ['bravo'];
        clock.elapse(const Duration(minutes: 7));
        clock.flushMicrotasks();
        expect(
          container.read(channelsProvider).requireValue.single.id,
          'bravo',
        );
        expect(offline.listeners, isNotEmpty);
        final latestReads = offline.latestReads;
        expect(latestReads, greaterThan(0));
        // Live updates work after recovery, and ordinary reconnects stay light.
        final live = offline.event(9, [
          ['h', 'bravo'],
          ['p', 'me'],
        ]);
        for (final listener in offline.listeners.toList()) {
          listener(live);
        }
        expect(
          container.read(channelsProvider).requireValue.single.lastMessageAt,
          DateTime.fromMillisecondsSinceEpoch(1000, isUtc: true),
        );
        offline.disconnect();
        offline.connect();
        clock.flushMicrotasks();
        expect(offline.latestReads, latestReads);
        expect(
          container.read(channelsProvider).requireValue.single.lastMessageAt,
          DateTime.fromMillisecondsSinceEpoch(1000, isUtc: true),
        );
        container.dispose();
        clock.elapse(const Duration(minutes: 2));
        expect(clock.pendingTimers, isEmpty);
      });

      container = _container(prefs, _SnapshotSession([], connected: false));
      addTearDown(container.dispose);
      expect(
        (await container.read(channelsProvider.future)).single.id,
        'bravo',
      );
    },
  );

  test(
    'removal retires both a parked fetch and later refreshes of that build',
    () async {
      SharedPreferences.setMockInitialValues({});
      final prefs = await SharedPreferences.getInstance();
      final storage = ChannelListSnapshotStorage(prefs);
      final session = _SnapshotSession(['alpha']);
      final container = _container(prefs, session);
      addTearDown(container.dispose);
      container.listen(channelsProvider, (_, _) {});
      await container.read(channelsProvider.future);
      await _settle();
      expect(storage.read('me', 'https://a.example'), isNotNull);
      final gate = Completer<void>();
      session.gate = gate;
      final refresh = container.read(channelsProvider.notifier).refresh();
      await _settle();
      await storage.remove('me', 'https://a.example');
      gate.complete();
      await refresh;
      session.gate = null;
      await container.read(channelsProvider.notifier).refresh();
      await _settle();
      expect(storage.read('me', 'https://a.example'), isNull);
    },
  );

  test(
    'relay and identity isolate restored lists and retire a parked refresh',
    () async {
      SharedPreferences.setMockInitialValues({});
      final prefs = await SharedPreferences.getInstance();
      final storage = ChannelListSnapshotStorage(prefs);
      for (final scope in [
        ('me', 'https://a.example', 'alpha'),
        ('other', 'https://a.example', 'bravo'),
        ('me', 'https://b.example', 'charlie'),
      ]) {
        await storage.write(
          storage.capture(scope.$1, scope.$2),
          ChannelListSnapshot(
            channels: [_channel(scope.$3).toJson()],
            hiddenDmIds: {},
          ),
        );
      }
      for (final scope in [
        ('me', 'https://a.example', 'alpha'),
        ('other', 'https://a.example', 'bravo'),
        ('me', 'https://b.example', 'charlie'),
      ]) {
        final container = _container(
          prefs,
          _SnapshotSession([], connected: false),
          pubkey: scope.$1,
          relay: scope.$2,
        );
        expect(
          (await container.read(channelsProvider.future)).single.id,
          scope.$3,
        );
        container.dispose();
      }
      final session = _SnapshotSession(['late-alpha'], connected: false);
      final gate = Completer<void>();
      session.gate = gate;
      final container = _container(prefs, session);
      addTearDown(container.dispose);
      container.listen(channelsProvider, (_, _) {});
      expect(
        (await container.read(channelsProvider.future)).single.id,
        'alpha',
      );
      session.connect();
      await _settle();
      expect(session.membershipRequests, 1);
      session.gate = null;
      session.disconnect();
      container
          .read(relayConfigProvider.notifier)
          .update(baseUrl: 'https://b.example');
      expect(
        (await container.read(channelsProvider.future)).single.id,
        'charlie',
      );
      gate.complete();
      await _settle();
      expect(
        container.read(channelsProvider).requireValue.single.id,
        'charlie',
      );
      expect(
        storage.read('me', 'https://a.example')!.channels.single['id'],
        'alpha',
      );
      expect(
        storage.read('me', 'https://b.example')!.channels.single['id'],
        'charlie',
      );
    },
  );
}

Channel _channel(String id) => Channel(
  id: id,
  name: id,
  channelType: 'stream',
  visibility: 'open',
  description: '',
  createdBy: 'me',
  createdAt: DateTime.utc(2026),
  memberCount: 1,
  isMember: true,
);

ProviderContainer _container(
  SharedPreferences prefs,
  _SnapshotSession session, {
  String pubkey = 'me',
  String relay = 'https://a.example',
  bool muted = false,
  ChannelMutesNotifier? mutes,
}) => ProviderContainer(
  retry: (_, _) => null,
  overrides: [
    savedPrefsProvider.overrideWithValue(prefs),
    relaySessionProvider.overrideWith(() => session),
    myPubkeyProvider.overrideWithValue(pubkey),
    relayConfigProvider.overrideWith(() => _Config(relay)),
    if (muted) channelMutesProvider.overrideWith(_Muted.new),
    if (mutes != null) channelMutesProvider.overrideWith(() => mutes),
  ],
);

class _Muted extends ChannelMutesNotifier {
  @override
  ChannelMutesState build() => const ChannelMutesState(
    isReady: true,
    store: ChannelMuteStore(
      channels: {'alpha': ChannelMuteEntry(muted: true, updatedAt: 1)},
    ),
  );
}

class _Config extends RelayConfigNotifier {
  _Config(this.relay);
  final String relay;
  @override
  RelayConfig build() => RelayConfig(baseUrl: relay);
}

class _SnapshotSession extends RelaySessionNotifier {
  _SnapshotSession(this.ids, {this.connected = true});
  List<String> ids;
  final bool connected;
  bool fail = false;
  bool latestTimeout = false;
  // Serves every channel as a DM.
  bool dm = false;
  bool failIds = false;
  bool deadlineIds = false;
  Completer<void>? gate;
  // Parks unread catch-up responses after they are computed.
  Completer<void>? unreadGate;
  int membershipRequests = 0;
  int latestReads = 0;
  final unreadMessages = <NostrEvent>[];
  final unreadFilters = <NostrFilter>[];
  final listeners = <void Function(NostrEvent)>[];
  final noticeListeners = <void Function(NostrEvent)>[];
  void emit(NostrEvent event) {
    for (final listener in [...listeners, ...noticeListeners]) {
      listener(event);
    }
  }

  @override
  SessionState build() => SessionState(
    status: connected ? SessionStatus.connected : SessionStatus.disconnected,
  );
  void connect() => state = const SessionState(status: SessionStatus.connected);
  void disconnect() =>
      state = const SessionState(status: SessionStatus.disconnected);
  NostrEvent event(int kind, List<List<String>> tags) => NostrEvent(
    id: '$kind-${tags.first.last}',
    pubkey: 'relay',
    createdAt: 1,
    kind: kind,
    tags: tags,
    content: '',
    sig: 'sig',
  );
  @override
  Future<List<NostrEvent>> queryRelay(
    List<NostrFilter> filters, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    if (filters.any((filter) => filter.limit == 20)) {
      latestReads++;
      if (latestTimeout) {
        throw RelayException(503, '{"error":"query timed out"}');
      }
    }
    if (filters.any((filter) => filter.limit == 1000)) {
      unreadFilters.addAll(filters);
      if (failIds && filters.any((f) => f.ids != null)) {
        if (deadlineIds) {
          throw RelayException(503, '{"error":"query timed out"}');
        }
        throw StateError('ID verification unavailable');
      }
      final results = <String, NostrEvent>{};
      for (final filter in filters) {
        final matches =
            unreadMessages
                .where(
                  (event) =>
                      filter.kinds.contains(event.kind) &&
                      (filter.ids == null || filter.ids!.contains(event.id)) &&
                      event.createdAt >= (filter.since ?? 0) &&
                      // The relay scopes a NIP-09 deletion without h by
                      // its target's stored channel.
                      (filter.tags['#h']!.contains(event.channelId) ||
                          (event.channelId == null &&
                              event.kind == EventKind.deletion)),
                )
                .toList()
              ..sort((a, b) => b.createdAt.compareTo(a.createdAt));
        for (final event in matches.take(filter.limit)) {
          results[event.id] = event;
        }
      }
      if (unreadGate case final gate?) await gate.future;
      return results.values.toList();
    }
    if (filters.singleOrNull?.kinds.contains(39002) == true) {
      if (filters.single.until != null) return [];
      membershipRequests++;
      final capturedIds = List<String>.of(ids);
      if (gate != null) await gate!.future;
      if (fail) throw StateError('offline');
      return [
        for (final id in capturedIds)
          event(39002, [
            ['d', id],
            ['p', 'me'],
          ]),
      ];
    }
    return [];
  }

  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    if (failIds && filter.ids != null) {
      throw StateError('ID verification fallback unavailable');
    }
    if (filter.kinds.contains(39000)) {
      return [
        for (final id in ids)
          event(39000, [
            ['d', id],
            ['name', id],
            ['t', dm ? 'dm' : 'stream'],
            ['public'],
          ]),
      ];
    }
    if (filter.kinds.contains(30622)) {
      return [
        event(30622, [
          ['d', 'hidden'],
          ['h', 'hidden-dm'],
        ]),
      ];
    }
    return [];
  }

  @override
  Future<void Function()> subscribeWithStatus(
    NostrFilter filter,
    void Function(NostrEvent) onEvent, {
    void Function(String message)? onClosed,
    required void Function(RelaySubscriptionStatus) onStatusChanged,
  }) => subscribe(filter, onEvent, onClosed: onClosed);

  @override
  Future<void Function()> subscribe(
    NostrFilter filter,
    void Function(NostrEvent) onEvent, {
    void Function(String)? onClosed,
  }) async {
    // The channel list notices are not a live channel subscription.
    final target = filter.kinds.contains(EventKind.memberAddedNotification)
        ? noticeListeners
        : listeners;
    target.add(onEvent);
    return () => target.remove(onEvent);
  }
}

Future<void> _settle() async {
  for (var i = 0; i < 30; i++) {
    await Future<void>.delayed(Duration.zero);
  }
}
