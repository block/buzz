import 'dart:async';
import 'dart:collection';
import 'dart:convert';

import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:buzz/features/channels/channel_messages_provider.dart';
import 'package:buzz/features/channels/pending_local_messages_provider.dart';
import 'package:buzz/features/channels/thread_replies_provider.dart';
import 'package:buzz/shared/relay/relay.dart';

void main() {
  test(
    'keeps live events that arrive while initial history is loading',
    () async {
      final relaySession = _RecordingRelaySessionNotifier();
      final container = _buildContainer(relaySession);
      addTearDown(container.dispose);

      container.read(channelMessagesProvider(_channelId));
      await relaySession.subscribed;

      relaySession.emit(_event(id: 'live', createdAt: 20));
      await _pumpEventQueue();

      expect(
        container
            .read(channelMessagesProvider(_channelId))
            .value
            ?.map((event) => event.id),
        ['live'],
      );

      relaySession.completeHistory([_event(id: 'history', createdAt: 10)]);
      await _pumpEventQueue();

      final messages = container
          .read(channelMessagesProvider(_channelId))
          .value!;
      expect(messages.map((event) => event.id), ['history', 'live']);
      expect(relaySession.operations, ['subscribe', 'query', 'fetch']);
      expect(relaySession.liveFilters.single.kinds, [
        ...EventKind.channelEventKinds,
        EventKind.channelThreadSummary,
      ]);
      expect(relaySession.liveFilters.single.tags['#h'], [_channelId]);
      expect(relaySession.liveFilters.single.limit, 200);
      expect(
        relaySession.queryFilters.first.kinds,
        EventKind.channelTimelineContentKinds,
      );
      expect(relaySession.queryFilters.first.tags['#h'], [_channelId]);
      expect(relaySession.queryFilters.first.extensions['top_level'], isTrue);
      expect(
        relaySession.historyFilters.first.kinds,
        EventKind.channelEventKinds,
      );
      expect(relaySession.historyFilters.first.tags['#h'], [_channelId]);
    },
  );

  test(
    'buffers a live thread summary until the initial window is installed',
    () async {
      final window = Completer<List<NostrEvent>>();
      final relaySession = _RecordingRelaySessionNotifier(
        queryResults: [window.future],
      );
      final container = _buildContainer(relaySession);
      addTearDown(container.dispose);

      container.read(channelMessagesProvider(_channelId));
      await relaySession.subscribed;
      final notifier = container.read(
        channelMessagesProvider(_channelId).notifier,
      );

      relaySession.emit(_summary(rootId: 'root', replyCount: 2));
      await _pumpEventQueue();
      expect(
        container.read(channelMessagesProvider(_channelId)).isLoading,
        isTrue,
      );

      window.complete([
        _event(id: 'root', createdAt: 10),
        _summary(rootId: 'root', replyCount: 1, createdAt: 10),
        _bounds(),
      ]);
      await _pumpEventQueue();

      expect(notifier.threadSummaries['root']?.replyCount, 2);
      expect(
        container
            .read(channelMessagesProvider(_channelId))
            .value
            ?.map((event) => event.id),
        ['root'],
      );
    },
  );

  test('still loads history when live subscription fails', () async {
    final relaySession = _RecordingRelaySessionNotifier(failSubscribe: true);
    final container = _buildContainer(relaySession);
    addTearDown(container.dispose);

    container.read(channelMessagesProvider(_channelId));
    await relaySession.subscribed;

    relaySession.completeHistory([_event(id: 'history', createdAt: 10)]);
    await _pumpEventQueue();

    final messages = container.read(channelMessagesProvider(_channelId)).value!;
    expect(messages.map((event) => event.id), ['history']);
    expect(relaySession.operations, ['subscribe', 'query', 'fetch']);
  });

  test(
    'keeps live messages when history sync fails after subscribing',
    () async {
      final relaySession = _RecordingRelaySessionNotifier();
      final container = _buildContainer(relaySession);
      addTearDown(container.dispose);

      container.read(channelMessagesProvider(_channelId));
      await relaySession.subscribed;

      relaySession.emit(_event(id: 'live', createdAt: 20));
      await _pumpEventQueue();

      relaySession.failHistory(Exception('history failed'));
      await _pumpEventQueue();

      final state = container.read(channelMessagesProvider(_channelId));
      expect(state.hasError, isFalse);
      expect(state.value?.map((event) => event.id), ['live']);
    },
  );

  test(
    'waits for initial history before publishing and preserves deep-link target',
    () async {
      final relaySession = _RecordingRelaySessionNotifier();
      final container = _buildContainer(relaySession);
      addTearDown(container.dispose);

      container.read(channelMessagesProvider(_channelId));
      await relaySession.subscribed;
      final notifier = container.read(
        channelMessagesProvider(_channelId).notifier,
      );

      final targetLoad = notifier.loadEventsById(const ['target']);
      relaySession.completeTargetHistory([_event(id: 'target', createdAt: 5)]);
      await targetLoad;

      expect(
        container.read(channelMessagesProvider(_channelId)).isLoading,
        isTrue,
      );

      relaySession.completeHistory([_event(id: 'history', createdAt: 10)]);
      await _pumpEventQueue();

      expect(
        container
            .read(channelMessagesProvider(_channelId))
            .value
            ?.map((event) => event.id),
        ['target', 'history'],
      );
    },
  );

  test(
    'adds and rolls back a local message in the websocket timeline',
    () async {
      final relaySession = _RecordingRelaySessionNotifier();
      final container = _buildContainer(relaySession);
      addTearDown(container.dispose);

      container.read(channelMessagesProvider(_channelId));
      await relaySession.subscribed;
      final notifier = container.read(
        channelMessagesProvider(_channelId).notifier,
      );

      notifier.addLocalMessage(_event(id: 'local', createdAt: 20));
      expect(
        container
            .read(channelMessagesProvider(_channelId))
            .value
            ?.map((event) => event.id),
        ['local'],
      );

      relaySession.completeHistory([_event(id: 'history', createdAt: 10)]);
      await _pumpEventQueue();

      // The initial history merge must retain a local row even if the relay's
      // history snapshot was taken before that outgoing event was durable.
      expect(
        container
            .read(channelMessagesProvider(_channelId))
            .value
            ?.map((event) => event.id),
        ['history', 'local'],
      );

      notifier.removeLocalMessage('local');
      expect(
        container
            .read(channelMessagesProvider(_channelId))
            .value
            ?.map((event) => event.id),
        ['history'],
      );
    },
  );

  test(
    'legacy websocket echo retires ownership without duplicating the row',
    () async {
      final relaySession = _RecordingRelaySessionNotifier();
      final container = _buildContainer(relaySession);
      addTearDown(container.dispose);

      container.read(channelMessagesProvider(_channelId));
      await relaySession.subscribed;
      final notifier = container.read(
        channelMessagesProvider(_channelId).notifier,
      );
      final local = _event(id: 'local', createdAt: 20);
      notifier.addLocalMessage(local);

      relaySession.emit(local);
      await _pumpEventQueue();

      expect(container.read(pendingLocalMessagesProvider(_channelId)), isEmpty);
      expect(
        container
            .read(channelMessagesProvider(_channelId))
            .value
            ?.map((event) => event.id),
        ['local'],
      );
    },
  );

  test('adds and rolls back a local message in the channel window', () async {
    final relaySession = _RecordingRelaySessionNotifier(
      queryResults: [
        [_event(id: 'history', createdAt: 10), _bounds()],
      ],
    );
    final container = _buildContainer(relaySession);
    addTearDown(container.dispose);

    container.read(channelMessagesProvider(_channelId));
    await relaySession.subscribed;
    await _pumpEventQueue();
    final notifier = container.read(
      channelMessagesProvider(_channelId).notifier,
    );

    notifier.addLocalMessage(_event(id: 'local', createdAt: 20));
    expect(
      container
          .read(channelMessagesProvider(_channelId))
          .value
          ?.map((event) => event.id),
      ['history', 'local'],
    );

    notifier.removeLocalMessage('local');
    expect(
      container
          .read(channelMessagesProvider(_channelId))
          .value
          ?.map((event) => event.id),
      ['history'],
    );
  });

  test(
    'live thread summary survives rolling back an unrelated local row',
    () async {
      final relaySession = _RecordingRelaySessionNotifier(
        queryResults: [
          [_event(id: 'root', createdAt: 10), _bounds()],
        ],
      );
      final container = _buildContainer(relaySession);
      addTearDown(container.dispose);

      container.read(channelMessagesProvider(_channelId));
      await relaySession.subscribed;
      await _pumpEventQueue();
      final notifier = container.read(
        channelMessagesProvider(_channelId).notifier,
      );
      notifier.addLocalMessage(_event(id: 'local', createdAt: 20));

      relaySession.emit(_summary(rootId: 'root', replyCount: 2));
      await _pumpEventQueue();
      expect(notifier.threadSummaries['root']?.replyCount, 2);

      notifier.removeLocalMessage('local');

      expect(notifier.threadSummaries['root']?.replyCount, 2);
      expect(
        container
            .read(channelMessagesProvider(_channelId))
            .value
            ?.map((event) => event.id),
        ['root'],
      );
    },
  );

  test('reconnect hydration cannot retain a rolled-back local row', () async {
    final relaySession = _RecordingRelaySessionNotifier(
      queryResults: [
        [_event(id: 'history', createdAt: 10), _bounds()],
        [_event(id: 'history', createdAt: 10), _bounds()],
      ],
    );
    final container = _buildContainer(relaySession);
    addTearDown(container.dispose);

    container.read(channelMessagesProvider(_channelId));
    await relaySession.subscribed;
    await _pumpEventQueue();
    final notifier = container.read(
      channelMessagesProvider(_channelId).notifier,
    );
    notifier.addLocalMessage(_event(id: 'local', createdAt: 20));

    relaySession.setConnected(false);
    await _pumpEventQueue();
    relaySession.setConnected(true);
    await _pumpEventQueue();
    expect(
      container
          .read(channelMessagesProvider(_channelId))
          .value
          ?.map((event) => event.id),
      ['history', 'local'],
    );

    notifier.removeLocalMessage('local');
    expect(
      container
          .read(channelMessagesProvider(_channelId))
          .value
          ?.map((event) => event.id),
      ['history'],
    );
  });

  test(
    'thread replies are inserted, deduped, and rolled back locally',
    () async {
      final relaySession = _RecordingRelaySessionNotifier(
        queryResults: [
          [_event(id: 'history', createdAt: 10), _bounds()],
          <NostrEvent>[],
          [
            _event(
              id: 'reply',
              createdAt: 20,
              extraTags: const [
                ['e', 'root', '', 'reply'],
              ],
            ),
          ],
        ],
      );
      final container = _buildContainer(relaySession);
      addTearDown(container.dispose);

      container.read(channelMessagesProvider(_channelId));
      await relaySession.subscribed;
      await _pumpEventQueue();
      const args = ThreadRepliesArgs(channelId: _channelId, rootId: 'root');
      container.read(threadRepliesWithLocalProvider(args));
      await _pumpEventQueue();
      final notifier = container.read(
        channelMessagesProvider(_channelId).notifier,
      );
      final reply = _event(
        id: 'reply',
        createdAt: 20,
        extraTags: const [
          ['e', 'root', '', 'reply'],
        ],
      );

      notifier.addLocalMessage(reply);
      expect(
        container
            .read(threadRepliesWithLocalProvider(args))
            .value
            ?.map((event) => event.id),
        ['reply'],
      );
      expect(
        container
            .read(channelMessagesProvider(_channelId))
            .value
            ?.map((event) => event.id),
        ['history'],
      );

      relaySession.emit(reply);
      await container.read(threadRepliesProvider(args).future);
      container.read(threadRepliesWithLocalProvider(args));
      await _pumpEventQueue();
      expect(
        container
            .read(threadRepliesWithLocalProvider(args))
            .value
            ?.map((event) => event.id),
        ['reply'],
      );
      expect(container.read(threadLocalRepliesProvider(args)), isEmpty);
      expect(container.read(pendingLocalMessagesProvider(_channelId)), isEmpty);

      final rejected = _event(
        id: 'rejected',
        createdAt: 21,
        extraTags: const [
          ['e', 'root', '', 'reply'],
        ],
      );
      notifier.addLocalMessage(rejected);
      notifier.removeLocalMessage('rejected');
      expect(
        container
            .read(threadRepliesWithLocalProvider(args))
            .value
            ?.map((event) => event.id),
        ['reply'],
      );
    },
  );

  test(
    'thread live echo keeps ownership until the authoritative refetch succeeds',
    () async {
      final relaySession = _RecordingRelaySessionNotifier(
        queryResults: [
          [_event(id: 'history', createdAt: 10), _bounds()],
          <NostrEvent>[],
          Exception('thread refetch failed'),
        ],
      );
      final container = _buildContainer(relaySession);
      addTearDown(container.dispose);

      container.read(channelMessagesProvider(_channelId));
      await relaySession.subscribed;
      await _pumpEventQueue();
      const args = ThreadRepliesArgs(channelId: _channelId, rootId: 'root');
      container.read(threadRepliesWithLocalProvider(args));
      await _pumpEventQueue();
      final notifier = container.read(
        channelMessagesProvider(_channelId).notifier,
      );
      final reply = _event(
        id: 'reply',
        createdAt: 20,
        extraTags: const [
          ['e', 'root', '', 'reply'],
        ],
      );
      notifier.addLocalMessage(reply);

      relaySession.emit(reply);
      await _pumpEventQueue();

      expect(container.read(pendingLocalMessagesProvider(_channelId)).keys, [
        'reply',
      ]);
      expect(
        container
            .read(threadLocalRepliesProvider(args))
            .map((event) => event.id),
        ['reply'],
      );
      expect(
        container
            .read(threadRepliesWithLocalProvider(args))
            .value
            ?.map((event) => event.id),
        ['reply'],
      );
    },
  );

  test(
    'successful never-echoed send releases ownership but keeps its row across reconnect',
    () async {
      final relaySession = _RecordingRelaySessionNotifier(
        queryResults: [
          [_event(id: 'history', createdAt: 10), _bounds()],
          [_event(id: 'history', createdAt: 10), _bounds()],
        ],
      );
      final container = _buildContainer(relaySession);
      addTearDown(container.dispose);

      container.read(channelMessagesProvider(_channelId));
      await relaySession.subscribed;
      await _pumpEventQueue();
      final notifier = container.read(
        channelMessagesProvider(_channelId).notifier,
      );
      notifier.addLocalMessage(_event(id: 'local', createdAt: 20));
      notifier.completeLocalMessage('local');

      expect(container.read(pendingLocalMessagesProvider(_channelId)), isEmpty);
      relaySession.setConnected(false);
      await _pumpEventQueue();
      relaySession.setConnected(true);
      await _pumpEventQueue();

      expect(container.read(pendingLocalMessagesProvider(_channelId)), isEmpty);
      expect(
        container
            .read(channelMessagesProvider(_channelId))
            .value
            ?.map((event) => event.id),
        ['history', 'local'],
      );
    },
  );

  test(
    'window dedupes echoes and orders rapid equal-time local sends',
    () async {
      final relaySession = _RecordingRelaySessionNotifier(
        queryResults: [
          [_event(id: 'history', createdAt: 10), _bounds()],
        ],
      );
      final container = _buildContainer(relaySession);
      addTearDown(container.dispose);

      container.read(channelMessagesProvider(_channelId));
      await relaySession.subscribed;
      await _pumpEventQueue();
      final notifier = container.read(
        channelMessagesProvider(_channelId).notifier,
      );
      notifier.addLocalMessage(_event(id: 'z-local', createdAt: 20));
      notifier.addLocalMessage(_event(id: 'a-local', createdAt: 20));
      relaySession.emit(_event(id: 'z-local', createdAt: 20));
      await _pumpEventQueue();

      expect(container.read(pendingLocalMessagesProvider(_channelId)).keys, [
        'a-local',
      ]);
      expect(
        container
            .read(channelMessagesProvider(_channelId))
            .value
            ?.map((event) => event.id),
        ['history', 'z-local', 'a-local'],
      );
    },
  );

  test('window pagination failures return false without exhausting', () async {
    final relaySession = _RecordingRelaySessionNotifier(
      queryResults: [
        [
          _event(id: 'head', createdAt: 20),
          _bounds(hasMore: true, cursorCreatedAt: 20, cursorId: 'head'),
        ],
        Exception('page failed'),
        [
          _event(id: 'older', createdAt: 10),
          _bounds(dTag: '${_channelId.toLowerCase()}:20:head'),
        ],
      ],
    );
    final container = _buildContainer(relaySession);
    addTearDown(container.dispose);

    container.read(channelMessagesProvider(_channelId));
    await relaySession.subscribed;
    await _pumpEventQueue();

    final notifier = container.read(
      channelMessagesProvider(_channelId).notifier,
    );
    expect(notifier.reachedOldest, isFalse);
    await expectLater(notifier.fetchOlder(), completion(isFalse));
    expect(notifier.reachedOldest, isFalse);

    await expectLater(notifier.fetchOlder(), completion(isTrue));
    expect(notifier.reachedOldest, isTrue);
    expect(
      container
          .read(channelMessagesProvider(_channelId))
          .value
          ?.map((e) => e.id),
      ['older', 'head'],
    );
  });
}

const _channelId = '11111111-1111-4111-8111-111111111111';

ProviderContainer _buildContainer(_RecordingRelaySessionNotifier relaySession) {
  return ProviderContainer(
    overrides: [relaySessionProvider.overrideWith(() => relaySession)],
  );
}

NostrEvent _event({
  required String id,
  required int createdAt,
  List<List<String>> extraTags = const [],
}) {
  return NostrEvent(
    id: id,
    pubkey: 'alice',
    createdAt: createdAt,
    kind: EventKind.streamMessageV2,
    tags: [
      ['h', _channelId],
      ...extraTags,
    ],
    content: id,
    sig: 'sig',
  );
}

NostrEvent _summary({
  required String rootId,
  required int replyCount,
  int createdAt = 20,
}) {
  return NostrEvent(
    id: 'summary-$rootId-$createdAt-$replyCount',
    pubkey: 'relay',
    createdAt: createdAt,
    kind: EventKind.channelThreadSummary,
    tags: [
      ['h', _channelId],
      ['e', rootId],
    ],
    content: jsonEncode({
      'reply_count': replyCount,
      'descendant_count': replyCount,
      'last_reply_at': 20,
      'participants': ['alice'],
    }),
    sig: 'sig',
  );
}

NostrEvent _bounds({
  bool hasMore = false,
  int? cursorCreatedAt,
  String? cursorId,
  String? dTag,
}) {
  return NostrEvent(
    id: 'bounds-$hasMore-${cursorId ?? dTag ?? 'none'}',
    pubkey: 'relay',
    createdAt: 0,
    kind: EventKind.channelWindowBounds,
    tags: [
      ['d', dTag ?? '${_channelId.toLowerCase()}:head'],
    ],
    content: jsonEncode({
      'has_more': hasMore,
      'next_cursor': hasMore
          ? {'created_at': cursorCreatedAt, 'id': cursorId}
          : null,
    }),
    sig: 'sig',
  );
}

Future<void> _pumpEventQueue() async {
  await Future<void>.delayed(Duration.zero);
  await Future<void>.delayed(Duration.zero);
}

class _RecordingRelaySessionNotifier extends RelaySessionNotifier {
  final bool failSubscribe;
  final Queue<Object> _queryResults;
  final List<String> operations = [];
  final List<NostrFilter> liveFilters = [];
  final List<NostrFilter> historyFilters = [];
  final List<NostrFilter> queryFilters = [];
  final List<void Function(NostrEvent)> _listeners = [];
  final Completer<void> _subscribed = Completer<void>();
  final Completer<List<NostrEvent>> _history = Completer<List<NostrEvent>>();
  final Queue<Completer<List<NostrEvent>>> _targetHistories = Queue();

  _RecordingRelaySessionNotifier({
    this.failSubscribe = false,
    List<Object> queryResults = const [],
  }) : _queryResults = Queue<Object>.of(queryResults);

  Future<void> get subscribed => _subscribed.future;

  @override
  SessionState build() => const SessionState(status: SessionStatus.connected);

  void setConnected(bool connected) {
    state = SessionState(
      status: connected ? SessionStatus.connected : SessionStatus.disconnected,
    );
  }

  @override
  Future<List<NostrEvent>> queryRelay(
    List<NostrFilter> filters, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    operations.add('query');
    queryFilters.addAll(filters);
    if (_queryResults.isEmpty) throw Exception('unsupported');
    final result = _queryResults.removeFirst();
    if (result is Exception) throw result;
    if (result is Future<List<NostrEvent>>) return await result;
    return (result as List<NostrEvent>).toList();
  }

  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) {
    operations.add('fetch');
    historyFilters.add(filter);
    if (filter.ids != null) {
      final completer = Completer<List<NostrEvent>>();
      _targetHistories.add(completer);
      return completer.future;
    }
    return _history.future;
  }

  @override
  Future<void Function()> subscribe(
    NostrFilter filter,
    void Function(NostrEvent) onEvent, {
    void Function(String message)? onClosed,
  }) async {
    operations.add('subscribe');
    liveFilters.add(filter);
    if (!_subscribed.isCompleted) {
      _subscribed.complete();
    }
    if (failSubscribe) {
      throw Exception('subscribe failed');
    }
    _listeners.add(onEvent);
    return () {
      _listeners.remove(onEvent);
    };
  }

  void emit(NostrEvent event) {
    for (final listener in List.of(_listeners)) {
      listener(event);
    }
  }

  void completeTargetHistory(List<NostrEvent> events) {
    _targetHistories.removeFirst().complete(events);
  }

  void completeHistory(List<NostrEvent> events) {
    if (!_history.isCompleted) {
      _history.complete(events);
    }
  }

  void failHistory(Object error) {
    if (!_history.isCompleted) {
      _history.completeError(error);
    }
  }
}
