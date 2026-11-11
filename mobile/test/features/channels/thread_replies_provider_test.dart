import 'dart:async';

import 'package:buzz/features/channels/pending_local_messages_provider.dart';
import 'package:buzz/features/channels/timeline_message.dart';
import 'package:buzz/features/channels/thread_replies_provider.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../helpers/thread_ids.dart';

class _FakeRelaySession extends RelaySessionNotifier {
  int queryCount = 0;
  bool honorDepthLimit = false;
  final filtersSeen = <NostrFilter>[];
  List<NostrEvent> replies = const [];
  // When set, each query answers with the next page instead of [replies].
  List<List<NostrEvent>>? pages;
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
    filtersSeen.addAll(filters);
    final gate = nextQueryGate;
    if (gate != null) {
      nextQueryGate = null;
      return gate.future;
    }
    final queued = pages;
    if (queued != null && queued.isNotEmpty) return queued.removeAt(0);
    if (honorDepthLimit) {
      final depth = filters.single.extensions['depth_limit'] as int;
      return replies.where((event) => event.createdAt <= depth).toList();
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
    ['e', threadIdRoot, '', 'reply'],
  ],
  content: 'reply $id',
  sig: '',
);

NostrEvent _edit(String id, String targetId, String content, int createdAt) =>
    NostrEvent(
      id: id,
      pubkey: 'bob',
      createdAt: createdAt,
      kind: EventKind.streamMessageEdit,
      tags: [
        const ['h', 'chan'],
        ['e', targetId],
      ],
      content: content,
      sig: '',
    );

void main() {
  const args = ThreadRepliesArgs(channelId: 'chan', rootId: threadIdRoot);

  test('an edit made before the thread opens folds into its reply', () async {
    final session = _FakeRelaySession()
      ..replies = [_reply('r1', 1000), _edit('e1', 'r1', 'typo fixed', 1100)];
    final container = ProviderContainer(
      overrides: [relaySessionProvider.overrideWith(() => session)],
    );
    addTearDown(container.dispose);
    container.listen(threadRepliesProvider(args), (_, _) {});
    final events = await container.read(threadRepliesProvider(args).future);
    expect(session.filtersSeen.single.extensions['include_aux'], isTrue);
    final reply = formatTimeline(events).single;
    expect((reply.id, reply.content, reply.edited), ('r1', 'typo fixed', true));
  });

  test('aux events neither end a scan page nor move its cursor', () async {
    // The relay sends a page's reply-marked rows, of any kind, before the aux
    // closure, and repeats the root's aux on every page. A reply-marked edit
    // is a row of its own page and aux on the others; a reply-marked reaction
    // is never a row.
    NostrEvent marked(String id, int createdAt, int kind) => NostrEvent(
      id: id,
      pubkey: 'bob',
      createdAt: createdAt,
      kind: kind,
      tags: const [
        ['h', 'chan'],
        ['e', threadIdRoot, '', 'reply'],
      ],
      content: '',
      sig: '',
    );
    List<(Object?, Object?)> cursors(_FakeRelaySession session) => [
      for (final filter in session.filtersSeen.skip(1))
        (
          filter.extensions['thread_cursor'],
          filter.extensions['thread_cursor_id'],
        ),
    ];
    final huddleRow = marked('h199', 1199, EventKind.huddleParticipantJoined);
    final markedEdit = marked('me', 1399, EventKind.streamMessageEdit);
    final markedReaction = marked('mr', 1500, EventKind.reaction);
    final rootEdit = _edit('e0', threadIdRoot, 'edited', 5000);
    final reactions = [
      for (var i = 0; i < 200; i++)
        NostrEvent(
          id: 'x$i',
          pubkey: 'bob',
          createdAt: 3000 + i,
          kind: EventKind.reaction,
          tags: const [
            ['h', 'chan'],
            ['e', 'last'],
          ],
          content: '+',
          sig: '',
        ),
    ];
    // A reply-marked edit right after a full page's rows tests the 200 cap.
    final rootAux = [markedEdit, markedReaction, rootEdit];
    final session = _FakeRelaySession()
      ..pages = [
        [for (var i = 0; i < 199; i++) _reply('r$i', 1000 + i), huddleRow],
        [for (var i = 0; i < 199; i++) _reply('s$i', 1200 + i), markedEdit],
        // A short page ends the scan even when its aux reaches the budget.
        [_reply('last', 2000), ...reactions],
      ].map((rows) => [...rows, ...rootAux]).toList();
    final result = await fetchCompleteThread(session, args);
    expect(cursors(session), [(1199, 'h199'), (1399, 'me')]);
    final replyIds = result.replies.map((event) => event.id);
    expect(replyIds, hasLength(401));
    expect(replyIds, containsAll(['h199', 'me', 'last']));
    expect(replyIds, isNot(contains('mr')));
    expect(result.aux.map((event) => event.id), [
      'me',
      'mr',
      'e0',
      ...reactions.map((event) => event.id),
    ]);

    // Reply-marked reactions to the root never fill a page.
    final reacted = _FakeRelaySession()
      ..replies = [
        for (var i = 0; i < 199; i++) _reply('r$i', 1000 + i),
        for (var i = 0; i < 200; i++) marked('mr$i', 500 + i, 7),
      ];
    final reactedResult = await fetchCompleteThread(reacted, args);
    expect(reacted.filtersSeen, hasLength(1));
    expect(reactedResult.replies, hasLength(199));

    // Reply-marked edits of the root are rows of an earlier page and the
    // root's aux on every page. After a short final page's rows they look
    // like a full page, but their cursor does not advance, so the scan ends.
    final rootEdits = [
      for (var i = 0; i < 200; i++)
        marked('me$i', 500 + i, EventKind.streamMessageEdit),
    ];
    final edited = _FakeRelaySession()
      ..pages = [
        [...rootEdits, ...rootEdits],
        [for (var i = 0; i < 10; i++) _reply('r$i', 1000 + i), ...rootEdits],
      ];
    final editedResult = await fetchCompleteThread(edited, args);
    expect(edited.filtersSeen, hasLength(2));
    expect(editedResult.replies, hasLength(210));

    // A reply-marked edit into another thread, in the aux after a short page,
    // is not a reply of this one.
    final otherRoot = 'b' * 64;
    final foreign = _FakeRelaySession()
      ..replies = [
        _reply('r0', 1000),
        NostrEvent(
          id: 'xe',
          pubkey: 'bob',
          createdAt: 1001,
          kind: EventKind.streamMessageEdit,
          tags: [
            const ['h', 'chan'],
            ['e', otherRoot, '', 'root'],
            ['e', otherRoot, '', 'reply'],
            const ['e', 'r0'],
          ],
          content: 'edit',
          sig: '',
        ),
      ];
    final foreignResult = await fetchCompleteThread(foreign, args);
    expect(foreignResult.replies.map((event) => event.id), ['r0']);
    expect(foreignResult.aux.map((event) => event.id), ['xe']);

    // The reply-count scan asks for replies alone.
    final counted = _FakeRelaySession()..replies = [_reply('r1', 1000)];
    await fetchCompleteThreadReplies(counted, args);
    expect(
      counted.filtersSeen.single.extensions,
      isNot(contains('include_aux')),
    );
  });

  test('complete scan includes a legal 80-deep reply chain', () async {
    final session = _FakeRelaySession()
      ..honorDepthLimit = true
      ..replies = [
        for (var depth = 1; depth <= 80; depth++)
          NostrEvent(
            id: 'reply-$depth',
            pubkey: 'bob',
            createdAt: depth,
            kind: EventKind.streamMessage,
            tags: [
              ['h', 'chan'],
              ['e', threadIdRoot, '', 'root'],
              [
                'e',
                depth == 1 ? threadIdRoot : 'reply-${depth - 1}',
                '',
                'reply',
              ],
            ],
            content: '',
            sig: '',
          ),
      ];
    final container = ProviderContainer(
      overrides: [relaySessionProvider.overrideWith(() => session)],
    );
    addTearDown(container.dispose);
    container.listen(threadRepliesProvider(args), (_, _) {});
    expect(
      await container.read(threadRepliesProvider(args).future),
      hasLength(80),
    );
  });

  test(
    'origin cursor includes epoch-zero events in an exhaustive scan',
    () async {
      final session = _FakeRelaySession()..replies = [_reply('epoch', 0)];
      final container = ProviderContainer(
        overrides: [relaySessionProvider.overrideWith(() => session)],
      );
      addTearDown(container.dispose);
      container.listen(threadRepliesProvider(args), (_, _) {});
      expect(
        (await container.read(
          threadRepliesProvider(args).future,
        )).single.createdAt,
        0,
      );
      expect(session.filtersSeen.single.extensions['thread_cursor'], -1);
      expect(
        session.filtersSeen.single.extensions['thread_cursor_id'],
        '0' * 64,
      );
    },
  );

  test(
    'disposes empty local overlays after their last listener leaves',
    () async {
      final container = ProviderContainer();
      addTearDown(container.dispose);
      final provider = threadLocalRepliesProvider(args);
      final subscription = container.listen(provider, (_, _) {});
      subscription.close();
      await container.pump();
      expect(container.exists(provider), isFalse);
    },
  );

  test('retains local replies without listeners until confirmation', () async {
    final container = ProviderContainer();
    addTearDown(container.dispose);
    final provider = threadLocalRepliesProvider(args);
    final notifier = container.read(provider.notifier);
    notifier.add(_reply('pending', 1000));
    await container.pump();
    expect(container.read(provider).single.id, 'pending');
    notifier.confirm({'pending'});
    await container.pump();
    expect(container.exists(provider), isFalse);
  });

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

  test(
    'local replies preserve cached authoritative replies after refresh failure',
    () async {
      final cached = _reply('cached', 1000);
      final local = _reply('local', 1001);
      final session = _FakeRelaySession()..replies = [cached];
      final container = ProviderContainer(
        retry: (_, _) => null,
        overrides: [relaySessionProvider.overrideWith(() => session)],
      );
      addTearDown(container.dispose);
      final combined = container.listen(
        threadRepliesWithLocalProvider(args),
        (_, _) {},
      );
      await container.read(threadRepliesProvider(args).future);
      container.read(threadLocalRepliesProvider(args).notifier).add(local);
      await container.pump();
      final refresh = Completer<List<NostrEvent>>();
      session.nextQueryGate = refresh;
      container.invalidate(threadRepliesProvider(args));
      await container.pump();
      expect(combined.read().value?.map((event) => event.id), [
        'cached',
        'local',
      ]);
      refresh.completeError(Exception('Refresh failed'));
      await container.pump();
      expect(container.read(threadRepliesProvider(args)).hasError, isTrue);
      expect(combined.read().value?.map((event) => event.id), [
        'cached',
        'local',
      ]);
      expect(container.read(threadLocalRepliesProvider(args)), [local]);
      session.replies = [cached, local];
      container.invalidate(threadRepliesProvider(args));
      await container.read(threadRepliesProvider(args).future);
      await container.pump();
      expect(combined.read().value?.map((event) => event.id), [
        'cached',
        'local',
      ]);
      await Future<void>.delayed(Duration.zero);
      expect(container.read(threadLocalRepliesProvider(args)), isEmpty);
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
