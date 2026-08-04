import 'dart:async';

import 'package:buzz/features/channels/pending_local_messages_provider.dart';
import 'package:buzz/features/channels/thread_replies_provider.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

void main() {
  test(
    'subscribes before history and keeps a reply delivered during the query',
    () async {
      final relaySession = _ThreadRelaySessionNotifier();
      final container = ProviderContainer(
        overrides: [relaySessionProvider.overrideWith(() => relaySession)],
      );
      addTearDown(container.dispose);

      const args = ThreadRepliesArgs(channelId: _channelId, rootId: _rootId);
      final subscription = container.listen(
        threadRepliesWithLocalProvider(args),
        (_, _) {},
        fireImmediately: true,
      );
      addTearDown(subscription.close);

      await relaySession.subscribed;
      await _pumpEventQueue();
      expect(relaySession.operations.take(2), ['subscribe', 'query']);
      final content = '@Naadir ${List.filled(2693, 'x').join()}';
      final reply = _reply(id: 'live-reply', createdAt: 20, content: content);

      relaySession.emit(reply);
      await _pumpEventQueue();

      expect(
        container
            .read(threadRepliesWithLocalProvider(args))
            .value
            ?.map((event) => event.id),
        ['live-reply'],
      );
      expect(
        container
            .read(threadRepliesWithLocalProvider(args))
            .value
            ?.single
            .content,
        content,
      );

      // A stale HTTP snapshot must not erase the websocket reply.
      relaySession.completeQuery(const []);
      await _pumpEventQueue();

      expect(
        container
            .read(threadRepliesWithLocalProvider(args))
            .value
            ?.map((event) => event.id),
        ['live-reply'],
      );
      expect(relaySession.liveFilters, hasLength(1));
      final filter = relaySession.liveFilters.single;
      expect(filter.kinds, EventKind.channelTimelineContentKinds);
      expect(filter.tags['#h'], [_channelId]);
      expect(filter.tags['#e'], [_rootId]);
      expect(filter.since, isNotNull);
    },
  );

  test(
    'retains the live subscription and accepts replay after a long reconnect',
    () async {
      final relaySession = _ThreadRelaySessionNotifier();
      relaySession.completeQuery(const []);
      final container = ProviderContainer(
        overrides: [relaySessionProvider.overrideWith(() => relaySession)],
      );
      addTearDown(container.dispose);

      const args = ThreadRepliesArgs(channelId: _channelId, rootId: _rootId);
      final subscription = container.listen(
        threadRepliesWithLocalProvider(args),
        (_, _) {},
        fireImmediately: true,
      );
      addTearDown(subscription.close);

      await relaySession.subscribed;
      await container.read(threadRepliesProvider(args).future);

      relaySession.emit(_reply(id: 'before-gap', createdAt: 100));
      await _pumpEventQueue();

      relaySession.setStatus(SessionStatus.disconnected);
      await _pumpEventQueue();
      relaySession.setStatus(SessionStatus.connected);
      await _pumpEventQueue();

      expect(relaySession.subscribeCount, 1);
      expect(relaySession.unsubscribeCount, 0);

      // RelaySession reuses this registered callback when replaying from
      // lastSeenCreatedAt - 5s. This event happened 20 seconds after the last
      // seen reply, so rebuilding a now-5s subscription would have lost it.
      relaySession.emitReplayed(_reply(id: 'during-long-gap', createdAt: 120));
      await _pumpEventQueue();

      expect(
        container
            .read(threadRepliesWithLocalProvider(args))
            .value
            ?.map((event) => event.id),
        ['before-gap', 'during-long-gap'],
      );
      expect(relaySession.subscribeCount, 1);
      expect(relaySession.unsubscribeCount, 0);
    },
  );

  test('a live relay echo confirms the optimistic local reply', () async {
    final relaySession = _ThreadRelaySessionNotifier();
    final container = ProviderContainer(
      overrides: [relaySessionProvider.overrideWith(() => relaySession)],
    );
    addTearDown(container.dispose);

    const args = ThreadRepliesArgs(channelId: _channelId, rootId: _rootId);
    final subscription = container.listen(
      threadRepliesWithLocalProvider(args),
      (_, _) {},
      fireImmediately: true,
    );
    addTearDown(subscription.close);

    await relaySession.subscribed;
    await _pumpEventQueue();
    final reply = _reply(id: 'local-echo', createdAt: 30);
    container.read(threadLocalRepliesProvider(args).notifier).add(reply);
    container
        .read(pendingLocalMessagesProvider(_channelId).notifier)
        .add(reply);

    relaySession.emit(reply);
    await _pumpEventQueue();

    expect(container.read(threadLocalRepliesProvider(args)), isEmpty);
    expect(container.read(pendingLocalMessagesProvider(_channelId)), isEmpty);
    expect(
      container
          .read(threadRepliesWithLocalProvider(args))
          .value
          ?.map((event) => event.id),
      ['local-echo'],
    );

    relaySession.completeQuery(const []);
    await _pumpEventQueue();
  });

  test('retries an initial live subscription failure', () async {
    const args = ThreadRepliesArgs(channelId: _channelId, rootId: _rootId);
    final relaySession = _ThreadRelaySessionNotifier(failuresBeforeSuccess: 1)
      ..completeQuery(const []);
    final container = ProviderContainer(
      overrides: [
        relaySessionProvider.overrideWith(() => relaySession),
        threadRepliesProvider(args).overrideWith(
          () => ThreadRepliesNotifier(args, retryBaseDelay: Duration.zero),
        ),
      ],
    );
    addTearDown(container.dispose);

    final subscription = container.listen(
      threadRepliesWithLocalProvider(args),
      (_, _) {},
      fireImmediately: true,
    );
    addTearDown(subscription.close);

    await _waitUntil(() => relaySession.subscribeCount == 2);
    expect(relaySession.activeListenerCount, 1);

    relaySession.emit(_reply(id: 'after-retry', createdAt: 40));
    await _pumpEventQueue();

    expect(
      container
          .read(threadRepliesWithLocalProvider(args))
          .value
          ?.map((event) => event.id),
      ['after-retry'],
    );
  });

  test('replaces a live subscription closed after readiness', () async {
    const args = ThreadRepliesArgs(channelId: _channelId, rootId: _rootId);
    final relaySession = _ThreadRelaySessionNotifier()..completeQuery(const []);
    final container = ProviderContainer(
      overrides: [
        relaySessionProvider.overrideWith(() => relaySession),
        threadRepliesProvider(args).overrideWith(
          () => ThreadRepliesNotifier(args, retryBaseDelay: Duration.zero),
        ),
      ],
    );
    addTearDown(container.dispose);

    final subscription = container.listen(
      threadRepliesWithLocalProvider(args),
      (_, _) {},
      fireImmediately: true,
    );
    addTearDown(subscription.close);

    await container.read(threadRepliesProvider(args).future);
    expect(relaySession.activeListenerCount, 1);

    relaySession.closeLiveSubscription('rate-limited: quota exceeded');
    await _waitUntil(() => relaySession.subscribeCount == 2);
    expect(relaySession.activeListenerCount, 1);

    relaySession.emit(_reply(id: 'after-close', createdAt: 50));
    await _pumpEventQueue();

    expect(
      container
          .read(threadRepliesWithLocalProvider(args))
          .value
          ?.map((event) => event.id),
      ['after-close'],
    );
  });
}

const _channelId = '11111111-1111-4111-8111-111111111111';
const _rootId = 'thread-root';

Future<void> _pumpEventQueue() async {
  await Future<void>.delayed(Duration.zero);
  await Future<void>.delayed(Duration.zero);
}

Future<void> _waitUntil(bool Function() predicate) async {
  for (var attempt = 0; attempt < 100; attempt++) {
    if (predicate()) return;
    await Future<void>.delayed(const Duration(milliseconds: 1));
  }
  fail('Condition was not met before timeout.');
}

NostrEvent _reply({
  required String id,
  required int createdAt,
  String? content,
}) {
  return NostrEvent(
    id: id,
    pubkey: 'fable',
    createdAt: createdAt,
    kind: EventKind.streamMessage,
    tags: const [
      ['h', _channelId],
      ['e', _rootId, '', 'reply'],
      ['p', 'owner'],
    ],
    content: content ?? id,
    sig: 'sig',
  );
}

class _ThreadRelaySessionNotifier extends RelaySessionNotifier {
  int failuresBeforeSuccess;
  final Completer<List<NostrEvent>> _query = Completer<List<NostrEvent>>();
  final Completer<void> _subscribed = Completer<void>();
  final List<_LiveRegistration> _registrations = [];
  final List<NostrFilter> liveFilters = [];
  final List<String> operations = [];
  int subscribeCount = 0;
  int unsubscribeCount = 0;

  _ThreadRelaySessionNotifier({this.failuresBeforeSuccess = 0});

  Future<void> get subscribed => _subscribed.future;
  int get activeListenerCount => _registrations.length;

  @override
  SessionState build() => const SessionState(status: SessionStatus.connected);

  void setStatus(SessionStatus status) {
    state = SessionState(status: status);
  }

  @override
  Future<List<NostrEvent>> queryRelay(
    List<NostrFilter> filters, {
    Duration timeout = const Duration(seconds: 8),
  }) {
    operations.add('query');
    return _query.future;
  }

  @override
  Future<void Function()> subscribe(
    NostrFilter filter,
    void Function(NostrEvent) onEvent, {
    void Function(String message)? onClosed,
  }) async {
    operations.add('subscribe');
    subscribeCount++;
    liveFilters.add(filter);
    if (!_subscribed.isCompleted) _subscribed.complete();
    if (failuresBeforeSuccess > 0) {
      failuresBeforeSuccess--;
      throw Exception('rate-limited: quota exceeded');
    }
    final registration = _LiveRegistration(onEvent, onClosed);
    _registrations.add(registration);
    return () {
      unsubscribeCount++;
      _registrations.remove(registration);
    };
  }

  void emit(NostrEvent event) {
    for (final registration in List.of(_registrations)) {
      registration.onEvent(event);
    }
  }

  void emitReplayed(NostrEvent event) => emit(event);

  void closeLiveSubscription(String message) {
    final registration = _registrations.removeAt(0);
    registration.onClosed?.call(message);
  }

  void completeQuery(List<NostrEvent> events) {
    if (!_query.isCompleted) _query.complete(events);
  }
}

class _LiveRegistration {
  final void Function(NostrEvent) onEvent;
  final void Function(String message)? onClosed;

  const _LiveRegistration(this.onEvent, this.onClosed);
}
