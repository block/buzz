import 'dart:async';

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
      // The auto-prefetch fires an extra fetchOlder because fewer than 15
      // displayable events were loaded. The deduped result sets _reachedOldest.
      expect(relaySession.operations, ['subscribe', 'fetch', 'fetch']);
      expect(
        relaySession.liveFilters.single.kinds,
        EventKind.channelEventKinds,
      );
      expect(relaySession.liveFilters.single.tags['#h'], [_channelId]);
      expect(relaySession.liveFilters.single.limit, 200);
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
    expect(relaySession.operations, ['subscribe', 'fetch', 'fetch']);
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

Future<void> _pumpEventQueue() async {
  await Future<void>.delayed(Duration.zero);
  await Future<void>.delayed(Duration.zero);
}

class _RecordingRelaySessionNotifier extends RelaySessionNotifier {
  final bool failSubscribe;
  final List<String> operations = [];
  final List<NostrFilter> liveFilters = [];
  final List<NostrFilter> historyFilters = [];
  final List<void Function(NostrEvent)> _listeners = [];
  final Completer<void> _subscribed = Completer<void>();
  final Completer<List<NostrEvent>> _history = Completer<List<NostrEvent>>();

  _RecordingRelaySessionNotifier({this.failSubscribe = false});

  Future<void> get subscribed => _subscribed.future;

  @override
  SessionState build() => const SessionState(status: SessionStatus.connected);

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
