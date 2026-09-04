import 'dart:async';

import 'package:buzz/features/channels/pending_local_messages_provider.dart';
import 'package:buzz/features/channels/thread_replies_provider.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

class _FakeRelaySession extends RelaySessionNotifier {
  int queryCount = 0;
  List<NostrEvent> replies = const [];
  Completer<List<NostrEvent>>? nextQueryGate;

  @override
  SessionState build() => const SessionState(status: SessionStatus.connected);

  void setStatus(SessionStatus status) {
    state = SessionState(status: status);
  }

  @override
  Future<List<NostrEvent>> queryRelay(
    List<NostrFilter> filters, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    queryCount++;
    final gate = nextQueryGate;
    if (gate != null) {
      nextQueryGate = null;
      return gate.future;
    }
    return replies;
  }
}

NostrEvent _reply(String id, int createdAt) => NostrEvent(
  id: id,
  pubkey: 'bob',
  createdAt: createdAt,
  kind: EventKind.streamMessage,
  tags: const [
    ['h', 'chan'],
    ['e', 'root', '', 'reply'],
  ],
  content: 'reply $id',
  sig: '',
);

void main() {
  const args = ThreadRepliesArgs(channelId: 'chan', rootId: 'root');

  (ProviderContainer, _FakeRelaySession, ProviderSubscription<Object?>)
  makeHarness(List<NostrEvent> initialReplies) {
    final fakeSession = _FakeRelaySession()..replies = initialReplies;
    final container = ProviderContainer(
      overrides: [relaySessionProvider.overrideWith(() => fakeSession)],
    );
    // An auto-disposed provider needs a listener to stay alive, mirroring an
    // open thread page. Creating it starts the first load, so the fake's
    // replies must be in place first.
    final subscription = container.listen(
      threadRepliesProvider(args),
      (_, _) {},
    );
    return (container, fakeSession, subscription);
  }

  test('thread replies keep desktop same-second id order', () {
    expect(
      mergeThreadEvents(
        [_reply('z', 1000)],
        [_reply('a', 1000), _reply('m', 1000)],
      ).map((event) => event.id),
      ['a', 'm', 'z'],
    );
  });

  test('does not refetch on the disconnect edge', () async {
    final (container, fakeSession, _) = makeHarness([_reply('r1', 1000)]);
    addTearDown(container.dispose);

    await container.read(threadRepliesProvider(args).future);
    final queriesAfterFirstLoad = fakeSession.queryCount;

    fakeSession.setStatus(SessionStatus.disconnected);
    await container.pump();

    expect(fakeSession.queryCount, queriesAfterFirstLoad);
  });

  test('refetches exactly once per reconnect edge', () async {
    final (container, fakeSession, _) = makeHarness([_reply('r1', 1000)]);
    addTearDown(container.dispose);

    final first = await container.read(threadRepliesProvider(args).future);
    expect(first.map((event) => event.id), ['r1']);
    final queriesAfterFirstLoad = fakeSession.queryCount;

    // A reply lands while the connection is down.
    fakeSession.replies = [_reply('r1', 1000), _reply('r2', 2000)];
    fakeSession.setStatus(SessionStatus.disconnected);
    await container.pump();
    fakeSession.setStatus(SessionStatus.connected);
    await container.pump();

    final second = await container.read(threadRepliesProvider(args).future);
    expect(second.map((event) => event.id), ['r1', 'r2']);
    expect(fakeSession.queryCount, queriesAfterFirstLoad + 1);
  });

  test(
    'does not refetch on session emissions that keep the same status',
    () async {
      final (container, fakeSession, _) = makeHarness([_reply('r1', 1000)]);
      addTearDown(container.dispose);

      await container.read(threadRepliesProvider(args).future);
      final queriesAfterFirstLoad = fakeSession.queryCount;

      // Same connected status, new state object (e.g. reconnectAttempt bump).
      fakeSession.setStatus(SessionStatus.connected);
      await container.pump();

      expect(fakeSession.queryCount, queriesAfterFirstLoad);
    },
  );

  test('keeps previous replies available while a refresh is pending', () async {
    final (container, fakeSession, _) = makeHarness([_reply('r1', 1000)]);
    addTearDown(container.dispose);

    await container.read(threadRepliesProvider(args).future);

    // Hold the reconnect refresh open and verify the old data still reads.
    final gate = Completer<List<NostrEvent>>();
    fakeSession.nextQueryGate = gate;
    fakeSession.setStatus(SessionStatus.disconnected);
    await container.pump();
    fakeSession.setStatus(SessionStatus.connected);
    await container.pump();

    final pending = container.read(threadRepliesProvider(args));
    expect(pending.isLoading, isTrue);
    expect(pending.value?.map((event) => event.id), ['r1']);

    gate.complete([_reply('r1', 1000), _reply('r2', 2000)]);
    final refreshed = await container.read(threadRepliesProvider(args).future);
    expect(refreshed.map((event) => event.id), ['r1', 'r2']);
  });

  test(
    'confirmation survives disposing the combined provider before its microtask',
    () async {
      final reply = _reply('r1', 1000);
      final query = Completer<List<NostrEvent>>();
      final fakeSession = _FakeRelaySession()..nextQueryGate = query;
      final container = ProviderContainer(
        overrides: [relaySessionProvider.overrideWith(() => fakeSession)],
      );
      addTearDown(container.dispose);
      container.read(threadLocalRepliesProvider(args).notifier).add(reply);
      container
          .read(pendingLocalMessagesProvider(args.channelId).notifier)
          .add(reply);

      late ProviderSubscription<AsyncValue<List<NostrEvent>>> subscription;
      subscription = container.listen(threadRepliesWithLocalProvider(args), (
        _,
        next,
      ) {
        if (next.value?.any((event) => event.id == reply.id) ?? false) {
          subscription.close();
        }
      });
      query.complete([reply]);
      await container.pump();
      await Future<void>.delayed(Duration.zero);

      expect(container.read(threadLocalRepliesProvider(args)), isEmpty);
      expect(
        container.read(pendingLocalMessagesProvider(args.channelId)),
        isEmpty,
      );
    },
  );

  test(
    'mounted thread renders a reply missed while disconnected after reconnect',
    () async {
      final fakeSession = _FakeRelaySession()..replies = [_reply('r1', 1000)];
      final container = ProviderContainer(
        overrides: [relaySessionProvider.overrideWith(() => fakeSession)],
      );
      addTearDown(container.dispose);
      final mountedThread = container.listen(
        threadRepliesWithLocalProvider(args),
        (_, _) {},
      );
      addTearDown(mountedThread.close);

      await container.read(threadRepliesProvider(args).future);
      expect(mountedThread.read().value?.map((event) => event.id), ['r1']);

      fakeSession.setStatus(SessionStatus.disconnected);
      await container.pump();
      fakeSession.replies = [_reply('r1', 1000), _reply('r2', 2000)];
      fakeSession.setStatus(SessionStatus.connected);
      await container.pump();
      await container.read(threadRepliesProvider(args).future);

      expect(mountedThread.read().value?.map((event) => event.id), [
        'r1',
        'r2',
      ]);
    },
  );

  test('reopening a disposed thread performs a fresh load', () async {
    final (container, fakeSession, subscription) = makeHarness([
      _reply('r1', 1000),
    ]);
    addTearDown(container.dispose);

    await container.read(threadRepliesProvider(args).future);
    final queriesAfterFirstLoad = fakeSession.queryCount;

    // Close the page: the auto-disposed query is torn down…
    subscription.close();
    await container.pump();

    // …so reopening loads fresh instead of serving a stale cache.
    fakeSession.replies = [_reply('r1', 1000), _reply('r2', 2000)];
    container.listen(threadRepliesProvider(args), (_, _) {});
    final reopened = await container.read(threadRepliesProvider(args).future);
    expect(reopened.map((event) => event.id), ['r1', 'r2']);
    expect(fakeSession.queryCount, queriesAfterFirstLoad + 1);
  });
}
