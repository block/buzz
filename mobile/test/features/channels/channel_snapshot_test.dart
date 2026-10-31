import 'dart:async';

import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channels_provider.dart';
import 'package:buzz/shared/channels/channel_list_snapshot.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/theme_provider.dart';
import 'package:fake_async/fake_async.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

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
        expect(
          offline.membershipRequests,
          3,
        ); // One bounded attempt per minute.
        expect(offline.listeners, isEmpty);
        // A slow attempt cannot accumulate overlapping periodic retries.
        final gate = Completer<void>();
        offline.gate = gate;
        clock.elapse(const Duration(minutes: 1));
        clock.flushMicrotasks();
        final parkedRequests = offline.membershipRequests;
        clock.elapse(const Duration(minutes: 2));
        clock.flushMicrotasks();
        expect(offline.membershipRequests, parkedRequests);
        gate.complete();
        clock.flushMicrotasks();
        offline.gate = null;
        expect(container.read(channelsProvider).hasError, isFalse);
        expect(container.read(channelsProvider).requireValue, hasLength(2));

        offline.fail = false;
        offline.ids = ['bravo'];
        clock.elapse(const Duration(minutes: 1));
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
}) => ProviderContainer(
  retry: (_, _) => null,
  overrides: [
    savedPrefsProvider.overrideWithValue(prefs),
    relaySessionProvider.overrideWith(() => session),
    myPubkeyProvider.overrideWithValue(pubkey),
    relayConfigProvider.overrideWith(() => _Config(relay)),
  ],
);

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
  Completer<void>? gate;
  int membershipRequests = 0;
  int latestReads = 0;
  final listeners = <void Function(NostrEvent)>[];
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
    if (filter.kinds.contains(39000)) {
      return [
        for (final id in ids)
          event(39000, [
            ['d', id],
            ['name', id],
            ['t', 'stream'],
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
  Future<void Function()> subscribe(
    NostrFilter filter,
    void Function(NostrEvent) onEvent, {
    void Function(String)? onClosed,
  }) async {
    listeners.add(onEvent);
    return () => listeners.remove(onEvent);
  }
}

Future<void> _settle() async {
  for (var i = 0; i < 30; i++) {
    await Future<void>.delayed(Duration.zero);
  }
}
