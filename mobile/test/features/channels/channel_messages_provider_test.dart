import 'dart:async';
import 'dart:collection';
import 'dart:convert';

import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:buzz/features/channels/channel_messages_provider.dart';
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
      expect(
        relaySession.liveFilters.single.kinds,
        EventKind.channelEventKinds,
      );
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

NostrEvent _event({required String id, required int createdAt}) {
  return NostrEvent(
    id: id,
    pubkey: 'alice',
    createdAt: createdAt,
    kind: EventKind.streamMessageV2,
    tags: const [
      ['h', _channelId],
    ],
    content: id,
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

  _RecordingRelaySessionNotifier({
    this.failSubscribe = false,
    List<Object> queryResults = const [],
  }) : _queryResults = Queue<Object>.of(queryResults);

  Future<void> get subscribed => _subscribed.future;

  @override
  SessionState build() => const SessionState(status: SessionStatus.connected);

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
    return (result as List<NostrEvent>).toList();
  }

  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) {
    operations.add('fetch');
    historyFilters.add(filter);
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
