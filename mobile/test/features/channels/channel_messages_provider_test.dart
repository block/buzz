import 'dart:async';
import 'dart:collection';
import 'dart:convert';

import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:buzz/features/channels/channel_messages_provider.dart';
import 'package:buzz/features/channels/pending_local_messages_provider.dart';
import 'package:buzz/features/channels/thread_replies_provider.dart';
import 'package:buzz/features/channels/timeline_message.dart';
import 'package:buzz/shared/relay/relay.dart';

void main() {
  test('live window without deep links does not rescan flattened ids', () async {
    var historyIdReads = 0;
    final history = _IdReadTrackingEvent(
      _event(id: 'history', createdAt: 10),
      onIdRead: () => historyIdReads++,
    );
    final relaySession = _RecordingRelaySessionNotifier(
      queryResults: [
        [history, _bounds()],
      ],
    );
    final container = _buildContainer(relaySession);
    addTearDown(container.dispose);
    container.read(channelMessagesProvider(_channelId));
    await relaySession.subscribed;
    await _pumpEventQueue();

    historyIdReads = 0;
    relaySession.emit(_event(id: 'live', createdAt: 20));

    // One read checks page membership; one builds the flattened window. The
    // distinct timestamps need no id tie-break when sorting. A redundant
    // deep-link merge would read the historical id a third time to build its
    // dedup set. Allow fewer reads if either required pass is optimized later.
    expect(historyIdReads, lessThanOrEqualTo(2));
    expect(
      container.read(channelMessagesProvider(_channelId)).value!.length,
      2,
    );
  });

  for (final retainDeepLink in [false, true]) {
    test(
      'live window keeps chronological order with retained deep link: $retainDeepLink',
      () async {
        final relaySession = _RecordingRelaySessionNotifier(
          queryResults: [
            [
              _event(id: 'newer-history', createdAt: 20),
              _event(id: 'older-history', createdAt: 10),
              _bounds(),
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
        if (retainDeepLink) {
          final load = notifier.loadEventsById(['deep-link']);
          relaySession.completeTargetHistory([
            _event(id: 'deep-link', createdAt: 5),
          ]);
          await load;
        }

        // Older live rows must insert in order, including the descending-id
        // tie-break within a second, on both sides of the deep-link fast path.
        relaySession.emit(_event(id: 'a-live', createdAt: 15));
        relaySession.emit(_event(id: 'z-live', createdAt: 15));
        expect(
          container
              .read(channelMessagesProvider(_channelId))
              .value!
              .map((event) => event.id),
          [
            if (retainDeepLink) 'deep-link',
            'older-history',
            'z-live',
            'a-live',
            'newer-history',
          ],
        );

        if (retainDeepLink) {
          notifier.releaseDeepLinkEvents(['deep-link']);
          relaySession.emit(_event(id: 'newest-live', createdAt: 30));
          expect(
            container
                .read(channelMessagesProvider(_channelId))
                .value!
                .map((event) => event.id),
            [
              'older-history',
              'z-live',
              'a-live',
              'newer-history',
              'newest-live',
            ],
          );
        }
      },
    );
  }

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
    'initial window hydration preserves equal-second live message order',
    () async {
      final window = Completer<List<NostrEvent>>();
      final relaySession = _RecordingRelaySessionNotifier(
        queryResults: [window.future],
      );
      final container = _buildContainer(relaySession);
      addTearDown(container.dispose);

      container.read(channelMessagesProvider(_channelId));
      await relaySession.subscribed;

      relaySession.emit(_event(id: 'z-live', createdAt: 20));
      relaySession.emit(_event(id: 'a-live', createdAt: 20));
      await _pumpEventQueue();
      expect(
        container
            .read(channelMessagesProvider(_channelId))
            .value
            ?.map((event) => event.id),
        ['z-live', 'a-live'],
      );

      window.complete([_bounds()]);
      await _pumpEventQueue();
      expect(
        container
            .read(channelMessagesProvider(_channelId))
            .value
            ?.map((event) => event.id),
        ['z-live', 'a-live'],
      );
    },
  );

  test(
    'websocket fallback uses desktop channel order for equal-second history',
    () async {
      final relaySession = _RecordingRelaySessionNotifier();
      final container = _buildContainer(relaySession);
      addTearDown(container.dispose);

      container.read(channelMessagesProvider(_channelId));
      await relaySession.subscribed;
      relaySession.completeHistory([
        _event(id: 'a-history', createdAt: 10),
        _event(id: 'z-history', createdAt: 10),
      ]);
      await _pumpEventQueue();

      expect(
        container
            .read(channelMessagesProvider(_channelId))
            .value
            ?.map((event) => event.id),
        ['z-history', 'a-history'],
      );
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

  test('retains historical and live Huddle end events in the window', () async {
    final relaySession = _RecordingRelaySessionNotifier(
      queryResults: [
        [
          _huddleEvent(
            id: 'ended-history',
            kind: EventKind.huddleEnded,
            createdAt: 20,
          ),
          _huddleEvent(
            id: 'started',
            kind: EventKind.huddleStarted,
            createdAt: 10,
          ),
          _bounds(),
        ],
      ],
    );
    final container = _buildContainer(relaySession);
    addTearDown(container.dispose);

    container.read(channelMessagesProvider(_channelId));
    await relaySession.subscribed;
    await _pumpEventQueue();

    expect(
      container
          .read(channelMessagesProvider(_channelId))
          .value
          ?.map((event) => event.id),
      ['started', 'ended-history'],
    );

    relaySession.emit(
      _huddleEvent(
        id: 'ended-live',
        kind: EventKind.huddleEnded,
        createdAt: 30,
      ),
    );
    await _pumpEventQueue();

    expect(
      container
          .read(channelMessagesProvider(_channelId))
          .value
          ?.map((event) => event.id),
      ['started', 'ended-history', 'ended-live'],
    );
  });

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

      final targetLoad = notifier.loadEventsById(const [
        'a-target',
        'z-target',
      ]);
      relaySession.completeTargetHistory([
        _event(id: 'a-target', createdAt: 10),
        _event(id: 'z-target', createdAt: 10),
      ]);
      await targetLoad;

      expect(
        container.read(channelMessagesProvider(_channelId)).isLoading,
        isTrue,
      );

      relaySession.completeHistory([_event(id: 'm-history', createdAt: 10)]);
      await _pumpEventQueue();

      expect(
        container
            .read(channelMessagesProvider(_channelId))
            .value
            ?.map((event) => event.id),
        ['z-target', 'm-history', 'a-target'],
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

  for (final nested in [false, true]) {
    test(
      'live reply settles a closed thread overlay, nested=$nested',
      () async {
        final relaySession = _RecordingRelaySessionNotifier(
          queryResults: [
            [
              if (nested)
                _event(
                  id: 'parent',
                  createdAt: 15,
                  extraTags: const [
                    ['e', 'root', '', 'reply'],
                  ],
                ),
              _event(id: 'root', createdAt: 10),
              _bounds(),
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
        const args = ThreadRepliesArgs(channelId: _channelId, rootId: 'root');
        final reply = _event(
          id: 'reply',
          createdAt: 20,
          extraTags: [
            if (nested) ['e', 'root', '', 'root'],
            ['e', nested ? 'parent' : 'root', '', 'reply'],
          ],
        );
        notifier.addLocalMessage(reply);
        notifier.completeLocalMessage(reply.id);
        relaySession.emit(reply);
        await _pumpEventQueue();
        expect(container.exists(threadLocalRepliesProvider(args)), isFalse);
        expect(
          container.read(pendingLocalMessagesProvider(_channelId)),
          isEmpty,
        );
        final entries = buildMainTimelineEntries(
          formatTimeline(
            container.read(channelMessagesProvider(_channelId)).value!,
          ),
        );
        expect(entries.single.summary?.replyCount, nested ? 2 : 1);
      },
    );
  }

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
      final threadSubscription = container.listen(
        threadRepliesWithLocalProvider(args),
        (_, _) {},
      );
      addTearDown(threadSubscription.close);
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
    'thread live echo settles ownership even when the refetch fails',
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
      final threadSubscription = container.listen(
        threadRepliesWithLocalProvider(args),
        (_, _) {},
      );
      addTearDown(threadSubscription.close);
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

      expect(container.read(pendingLocalMessagesProvider(_channelId)), isEmpty);
      expect(container.read(threadLocalRepliesProvider(args)), isEmpty);
      expect(
        container
            .read(channelMessagesProvider(_channelId))
            .value!
            .map((event) => event.id),
        ['history', 'reply'],
      );
    },
  );

  test(
    'websocket fallback refetches an open thread when a reply arrives live',
    () async {
      final relaySession = _RecordingRelaySessionNotifier(
        queryResults: [
          Exception('channel window unavailable'),
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
      relaySession.completeHistory([_event(id: 'history', createdAt: 10)]);
      await _pumpEventQueue();
      expect(relaySession.operations, ['subscribe', 'query', 'fetch']);

      const args = ThreadRepliesArgs(channelId: _channelId, rootId: 'root');
      final subscription = container.listen(
        threadRepliesProvider(args),
        (_, _) {},
      );
      addTearDown(subscription.close);
      expect(await container.read(threadRepliesProvider(args).future), isEmpty);

      relaySession.emit(
        _event(
          id: 'reply',
          createdAt: 20,
          extraTags: const [
            ['e', 'root', '', 'reply'],
          ],
        ),
      );
      await _pumpEventQueue();

      expect(
        (await container.read(
          threadRepliesProvider(args).future,
        )).map((event) => event.id),
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

  test(
    'a live reply reaches the store so its parent badge can count it',
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

      relaySession.emit(
        _event(
          id: 'reply',
          createdAt: 20,
          extraTags: const [
            ['e', 'root', '', 'reply'],
          ],
        ),
      );
      await _pumpEventQueue();

      // The reply is retained as the local half of the summary merge. It is
      // filtered out of the main timeline by `buildMainTimelineEntries`, which
      // owns reply visibility.
      expect(
        container
            .read(channelMessagesProvider(_channelId))
            .value
            ?.map((event) => event.id),
        ['root', 'reply'],
      );
      expect(
        buildMainTimelineEntries(
          formatTimeline(
            container.read(channelMessagesProvider(_channelId)).value!,
          ),
          relaySummaries: container
              .read(channelMessagesProvider(_channelId).notifier)
              .threadSummaries,
        ).map((entry) => entry.message.id),
        ['root'],
      );
    },
  );

  for (final deleted in [false, true]) {
    test(
      'confirmed reply evidence survives stale reconnect (deleted: $deleted)',
      () async {
        final root = _event(id: 'root', createdAt: 10);
        final reply = _event(
          id: 'reply',
          createdAt: 20,
          extraTags: const [
            ['e', 'root', '', 'reply'],
          ],
        );
        final relaySession = _RecordingRelaySessionNotifier(
          queryResults: [
            [root, _bounds()],
            [root, _bounds()],
          ],
        );
        final container = _buildContainer(relaySession);
        addTearDown(container.dispose);
        final subscription = container.listen(
          channelMessagesProvider(_channelId),
          (_, _) {},
          fireImmediately: true,
        );
        addTearDown(subscription.close);
        await relaySession.subscribed;
        await _pumpEventQueue();
        final notifier = container.read(
          channelMessagesProvider(_channelId).notifier,
        );
        // The thread query confirmed the reply, but its live echo and recount
        // were missed. A bounded-staleness reconnect window can still omit it.
        notifier.cacheConfirmedThreadReplies([reply]);
        if (deleted) {
          relaySession.emit(
            NostrEvent(
              id: 'delete-reply',
              pubkey: 'alice',
              createdAt: 30,
              kind: EventKind.deletion,
              tags: const [
                ['h', _channelId],
                ['e', 'reply'],
              ],
              content: '',
              sig: 'sig',
            ),
          );
          await _pumpEventQueue();
        }
        relaySession.setConnected(false);
        await _pumpEventQueue();
        relaySession.setConnected(true);
        await _pumpEventQueue();
        // A subsequent live event rebuilds state from the window store.
        relaySession.emit(_event(id: 'unrelated', createdAt: 40));
        await _pumpEventQueue();
        final entries = buildMainTimelineEntries(
          formatTimeline(
            container.read(channelMessagesProvider(_channelId)).value!,
          ),
          relaySummaries: notifier.threadSummaries,
        );
        final rootEntry = entries.singleWhere(
          (entry) => entry.message.id == 'root',
        );
        if (deleted) {
          expect(rootEntry.summary, isNull);
        } else {
          expect(rootEntry.summary?.replyCount, 1);
        }
      },
    );
  }

  for (final deletionMarkerAvailable in [false, true]) {
    for (final lateArrival in [false, true]) {
      test(
        'complete thread scan removes absent replies (marker: $deletionMarkerAvailable, late arrival: $lateArrival)',
        () async {
          final query = Completer<List<NostrEvent>>();
          final root = _event(id: 'root', createdAt: 10);
          final relaySession = _RecordingRelaySessionNotifier(
            queryResults: [
              [root, _bounds()],
              [root, _bounds()],
              query.future,
              <NostrEvent>[
                if (deletionMarkerAvailable)
                  NostrEvent(
                    id: 'offline-deletion',
                    pubkey: 'alice',
                    createdAt: 25,
                    kind: EventKind.deletion,
                    tags: const [
                      ['h', _channelId],
                      ['e', 'deleted-offline'],
                    ],
                    content: '',
                    sig: 'sig',
                  ),
              ],
            ],
          );
          final container = _buildContainer(relaySession);
          addTearDown(container.dispose);
          final channelSubscription = container.listen(
            channelMessagesProvider(_channelId),
            (_, _) {},
            fireImmediately: true,
          );
          addTearDown(channelSubscription.close);
          await _pumpEventQueue();
          final notifier = container.read(
            channelMessagesProvider(_channelId).notifier,
          );
          notifier.cacheConfirmedThreadReplies([
            _event(
              id: 'deleted-offline',
              createdAt: 20,
              extraTags: const [
                ['e', 'root', '', 'reply'],
              ],
            ),
          ]);
          relaySession.setConnected(false);
          await _pumpEventQueue();
          relaySession.setConnected(true);
          await _pumpEventQueue();
          const args = ThreadRepliesArgs(channelId: _channelId, rootId: 'root');
          final threadSubscription = container.listen(
            threadRepliesProvider(args),
            (_, _) {},
          );
          addTearDown(threadSubscription.close);
          await _pumpEventQueue();
          // Another confirmation arrives after the query starts. Its absence from
          // that query must not erase it along with the old cached reply.
          if (lateArrival) {
            notifier.cacheConfirmedThreadReplies([
              _event(
                id: 'new-arrival',
                createdAt: 30,
                extraTags: const [
                  ['e', 'root', '', 'reply'],
                ],
              ),
            ]);
          }
          query.complete([]);
          await container.read(threadRepliesProvider(args).future);
          await _pumpEventQueue();
          relaySession.emit(_event(id: 'unrelated', createdAt: 40));
          await _pumpEventQueue();
          final events = container
              .read(channelMessagesProvider(_channelId))
              .value!;
          expect(formatTimeline(events).map((event) => event.id), [
            'root',
            if (lateArrival) 'new-arrival',
            'unrelated',
          ]);
          final merged = mergeThreadEvents(
            container.read(threadRepliesProvider(args)).value!,
            events,
          );
          expect(
            formatTimeline(
              merged,
            ).any((event) => event.id == 'deleted-offline'),
            isFalse,
          );
          final rootEntry = buildMainTimelineEntries(
            formatTimeline(events),
            relaySummaries: notifier.threadSummaries,
          ).singleWhere((entry) => entry.message.id == 'root');
          final expectedCount = lateArrival ? 1 : 0;
          expect(
            rootEntry.summary?.replyCount,
            expectedCount == 0 ? null : expectedCount,
          );
        },
      );
    }
  }
  test(
    'query-discovered replies enable the root without a local overlay',
    () async {
      final reply = _event(
        id: 'discovered',
        createdAt: 20,
        extraTags: const [
          ['e', 'root', '', 'reply'],
        ],
      );
      final session = _RecordingRelaySessionNotifier(
        queryResults: [
          [_event(id: 'root', createdAt: 10), _bounds()],
          [reply],
        ],
      );
      final container = _buildContainer(session);
      addTearDown(container.dispose);
      container.listen(channelMessagesProvider(_channelId), (_, _) {});
      await _pumpEventQueue();
      const args = ThreadRepliesArgs(channelId: _channelId, rootId: 'root');
      final thread = container.listen(threadRepliesProvider(args), (_, _) {});
      await container.read(threadRepliesProvider(args).future);
      thread.close();
      await _pumpEventQueue();
      final notifier = container.read(
        channelMessagesProvider(_channelId).notifier,
      );
      final entries = buildMainTimelineEntries(
        formatTimeline(
          container.read(channelMessagesProvider(_channelId)).value!,
        ),
        relaySummaries: notifier.threadSummaries,
      );
      expect(entries.single.summary?.replyCount, 1);
      expect(container.exists(threadLocalRepliesProvider(args)), isFalse);
    },
  );

  test('reconnect evicts reply evidence outside the newest window', () async {
    final session = _RecordingRelaySessionNotifier(
      queryResults: [
        [_event(id: 'old-root', createdAt: 10), _bounds()],
        [_event(id: 'new-root', createdAt: 30), _bounds()],
        [_event(id: 'new-root', createdAt: 30), _bounds()],
      ],
    );
    final container = _buildContainer(session);
    addTearDown(container.dispose);
    container.listen(channelMessagesProvider(_channelId), (_, _) {});
    await _pumpEventQueue();
    final notifier = container.read(
      channelMessagesProvider(_channelId).notifier,
    );
    notifier.cacheConfirmedThreadReplies([
      _event(
        id: 'old-reply',
        createdAt: 20,
        extraTags: const [
          ['e', 'old-root', '', 'reply'],
        ],
      ),
    ]);
    for (var cycle = 0; cycle < 2; cycle++) {
      session.setConnected(false);
      await _pumpEventQueue();
      session.setConnected(true);
      await _pumpEventQueue();
      expect(notifier.cachedThreadReplyIds('old-root'), isEmpty);
      session.emit(
        _event(id: 'window-publication-$cycle', createdAt: 50 + cycle),
      );
      await _pumpEventQueue();
      expect(
        container
            .read(channelMessagesProvider(_channelId))
            .value!
            .any((event) => event.id == 'old-reply'),
        isFalse,
      );
      notifier.cacheConfirmedThreadReplies([
        _event(
          id: 'new-reply',
          createdAt: 40,
          extraTags: const [
            ['e', 'new-root', '', 'reply'],
          ],
        ),
      ]);
    }
    expect(notifier.cachedThreadReplyIds('new-root'), {'new-reply'});
  });

  for (final nested in [false, true]) {
    for (final fetched in [false, true]) {
      test(
        'deletion settles retained local overlay (nested: $nested, fetched: $fetched)',
        () async {
          final deletion = NostrEvent(
            id: 'delete',
            pubkey: 'alice',
            createdAt: 30,
            kind: EventKind.deletion,
            tags: const [
              ['h', _channelId],
              ['e', 'local'],
            ],
            content: '',
            sig: 'sig',
          );
          final session = _RecordingRelaySessionNotifier(
            queryResults: [
              [_event(id: 'root', createdAt: 10), _bounds()],
              <NostrEvent>[],
              [deletion],
            ],
          );
          final container = _buildContainer(session);
          addTearDown(container.dispose);
          container.listen(channelMessagesProvider(_channelId), (_, _) {});
          await _pumpEventQueue();
          final notifier = container.read(
            channelMessagesProvider(_channelId).notifier,
          );
          notifier.addLocalMessage(
            _event(
              id: 'local',
              createdAt: 20,
              extraTags: [
                if (nested) ['e', 'root', '', 'root'],
                ['e', nested ? 'parent' : 'root', '', 'reply'],
              ],
            ),
          );
          notifier.completeLocalMessage('local');
          const args = ThreadRepliesArgs(channelId: _channelId, rootId: 'root');
          if (fetched) {
            container.listen(threadRepliesProvider(args), (_, _) {});
            await container.read(threadRepliesProvider(args).future);
          } else {
            session.emit(deletion);
          }
          await _pumpEventQueue();
          expect(container.exists(threadLocalRepliesProvider(args)), isFalse);
          expect(notifier.cachedThreadReplyIds('root'), isEmpty);
          final entries = buildMainTimelineEntries(
            formatTimeline(
              container.read(channelMessagesProvider(_channelId)).value!,
            ),
            relaySummaries: notifier.threadSummaries,
          );
          expect(entries.single.summary, isNull);
        },
      );
    }
  }

  test(
    'complete scan clears all missing retained replies beyond the old deletion cap',
    () async {
      final replies = [
        for (var i = 0; i < 600; i++)
          _event(
            id: 'reply-$i',
            createdAt: 20 + i,
            extraTags: const [
              ['e', 'root', '', 'reply'],
            ],
          ),
      ];
      final session = _RecordingRelaySessionNotifier(
        queryResults: [
          [_event(id: 'root', createdAt: 10), _bounds()],
          replies.take(200).toList(),
          replies.skip(200).take(200).toList(),
          replies.skip(400).toList(),
          <NostrEvent>[],
          <NostrEvent>[],
        ],
      );
      final container = _buildContainer(session);
      addTearDown(container.dispose);
      container.listen(channelMessagesProvider(_channelId), (_, _) {});
      await _pumpEventQueue();
      const args = ThreadRepliesArgs(channelId: _channelId, rootId: 'root');
      container.listen(threadRepliesProvider(args), (_, _) {});
      expect(
        await container.read(threadRepliesProvider(args).future),
        hasLength(600),
      );
      final notifier = container.read(
        channelMessagesProvider(_channelId).notifier,
      );
      expect(notifier.cachedThreadReplyIds('root'), hasLength(256));
      expect(
        container.read(channelMessagesProvider(_channelId)).value,
        hasLength(257),
      );
      container.invalidate(threadRepliesProvider(args));
      await container.read(threadRepliesProvider(args).future);
      final deletionFilters = session.queryFilters
          .where((filter) => filter.kinds.contains(EventKind.deletion))
          .toList();
      expect(deletionFilters, isEmpty);
      expect(notifier.threadSummaries['root']?.descendantCount, 0);
      expect(
        formatTimeline(
          container.read(channelMessagesProvider(_channelId)).value!,
        ).map((event) => event.id),
        ['root'],
      );
    },
  );

  test(
    'reply payload cache has a channel-wide bound across many roots',
    () async {
      final session = _RecordingRelaySessionNotifier(
        queryResults: [
          [_event(id: 'root', createdAt: 10), _bounds()],
        ],
      );
      final container = _buildContainer(session);
      addTearDown(container.dispose);
      container.listen(channelMessagesProvider(_channelId), (_, _) {});
      await _pumpEventQueue();
      final notifier = container.read(
        channelMessagesProvider(_channelId).notifier,
      );
      notifier.cacheConfirmedThreadReplies([
        for (var root = 0; root < 12; root++)
          for (var i = 0; i < 300; i++)
            _event(
              id: '$root-$i',
              createdAt: 20 + root * 300 + i,
              extraTags: [
                ['e', 'root-$root', '', 'reply'],
              ],
            ),
      ]);
      final events = container.read(channelMessagesProvider(_channelId)).value!;
      expect(
        events.where((event) => event.threadReference.parentId != null),
        hasLength(2048),
      );
      for (var root = 0; root < 12; root++) {
        expect(
          notifier.cachedThreadReplyIds('root-$root').length,
          lessThanOrEqualTo(256),
        );
      }
    },
  );

  test(
    'pinned off-window root retains query-discovered reply evidence',
    () async {
      final session = _RecordingRelaySessionNotifier(
        queryResults: [
          [_event(id: 'newest', createdAt: 100), _bounds()],
          [
            _event(
              id: 'old-reply',
              createdAt: 20,
              extraTags: const [
                ['e', 'old-root', '', 'reply'],
              ],
            ),
          ],
        ],
      );
      final container = _buildContainer(session);
      addTearDown(container.dispose);
      container.listen(channelMessagesProvider(_channelId), (_, _) {});
      await _pumpEventQueue();
      final notifier = container.read(
        channelMessagesProvider(_channelId).notifier,
      );
      final load = notifier.loadEventsById(['old-root']);
      session.completeTargetHistory([_event(id: 'old-root', createdAt: 10)]);
      await load;
      const args = ThreadRepliesArgs(channelId: _channelId, rootId: 'old-root');
      final thread = container.listen(threadRepliesProvider(args), (_, _) {});
      await container.read(threadRepliesProvider(args).future);
      thread.close();
      await _pumpEventQueue();
      session.emit(_event(id: 'live', createdAt: 110));
      final entries = buildMainTimelineEntries(
        formatTimeline(
          container.read(channelMessagesProvider(_channelId)).value!,
        ),
        relaySummaries: notifier.threadSummaries,
      );
      expect(
        entries
            .singleWhere((entry) => entry.message.id == 'old-root')
            .summary
            ?.replyCount,
        1,
      );
    },
  );

  for (final source in ['ack', 'live']) {
    test(
      '$source replies obey payload bounds without reopening threads',
      () async {
        final session = _RecordingRelaySessionNotifier(
          queryResults: [
            [_event(id: 'root', createdAt: 10), _bounds()],
          ],
        );
        final container = _buildContainer(session);
        addTearDown(container.dispose);
        container.listen(channelMessagesProvider(_channelId), (_, _) {});
        await _pumpEventQueue();
        final notifier = container.read(
          channelMessagesProvider(_channelId).notifier,
        );
        for (var root = 0; root < 9; root++) {
          for (var i = 0; i < 300; i++) {
            final reply = _event(
              id: '$root-$i',
              createdAt: 20 + root * 300 + i,
              extraTags: [
                ['e', 'root-$root', '', 'reply'],
              ],
            );
            if (source == 'ack') {
              notifier.addLocalMessage(reply);
              notifier.completeLocalMessage(reply.id);
            } else {
              session.emit(reply);
            }
          }
          expect(
            notifier.cachedThreadReplyIds('root-$root').length,
            lessThanOrEqualTo(256),
          );
        }
        await _pumpEventQueue();
        expect(
          container
              .read(channelMessagesProvider(_channelId))
              .value!
              .where((event) => event.threadReference.parentId != null),
          hasLength(2048),
        );
        expect(
          container.read(pendingLocalMessagesProvider(_channelId)),
          isEmpty,
        );
        for (var root = 0; root < 9; root++) {
          expect(
            container.exists(
              threadLocalRepliesProvider(
                ThreadRepliesArgs(channelId: _channelId, rootId: 'root-$root'),
              ),
            ),
            isFalse,
          );
        }
      },
    );
  }

  for (final count in [257, 600]) {
    test(
      'complete scan preserves $count reply summary through reconnect and reopen',
      () async {
        final replies = [
          for (var i = 0; i < count; i++)
            _event(
              id: 'reply-$i',
              createdAt: 20 + i,
              extraTags: const [
                ['e', 'root', '', 'reply'],
              ],
            ),
        ];
        final pages = <List<NostrEvent>>[];
        for (var start = 0; start < count; start += 200) {
          pages.add(replies.skip(start).take(200).toList());
        }
        if (count % 200 == 0) pages.add([]);
        final window = [_event(id: 'root', createdAt: 10), _bounds()];
        final session = _RecordingRelaySessionNotifier(
          queryResults: [window, ...pages, window, ...pages],
        );
        final container = _buildContainer(session);
        addTearDown(container.dispose);
        container.listen(channelMessagesProvider(_channelId), (_, _) {});
        await _pumpEventQueue();
        const args = ThreadRepliesArgs(channelId: _channelId, rootId: 'root');
        final thread = container.listen(threadRepliesProvider(args), (_, _) {});
        expect(
          await container.read(threadRepliesProvider(args).future),
          hasLength(count),
        );
        thread.close();
        await _pumpEventQueue();
        final notifier = container.read(
          channelMessagesProvider(_channelId).notifier,
        );
        void checkSummary() {
          final entries = buildMainTimelineEntries(
            formatTimeline(
              container.read(channelMessagesProvider(_channelId)).value!,
            ),
            relaySummaries: notifier.threadSummaries,
          );
          expect(
            entries
                .singleWhere((entry) => entry.message.id == 'root')
                .summary
                ?.replyCount,
            count,
          );
          expect(notifier.cachedThreadReplyIds('root'), hasLength(256));
          expect(notifier.threadSummaries['root']?.lastReplyAt, 19 + count);
          expect(notifier.threadSummaries['root']?.participantPubkeys, [
            'alice',
          ]);
        }

        checkSummary();
        session.setConnected(false);
        await _pumpEventQueue();
        session.setConnected(true);
        await _pumpEventQueue();
        session.emit(_event(id: 'unrelated', createdAt: 1000));
        checkSummary();
        final reopened = container.listen(
          threadRepliesProvider(args),
          (_, _) {},
        );
        expect(
          await container.read(threadRepliesProvider(args).future),
          hasLength(count),
        );
        checkSummary();
        reopened.close();
        final scans = session.queryFilters
            .where((filter) => filter.extensions.containsKey('depth_limit'))
            .toList();
        expect(scans.first.extensions['thread_cursor'], -1);
        expect(scans.first.extensions['thread_cursor_id'], '0' * 64);
      },
    );
  }

  test(
    'an unacknowledged reply needs explicit deletion proof after a complete scan',
    () async {
      final deletion = NostrEvent(
        id: 'delete-pending',
        pubkey: 'alice',
        createdAt: 30,
        kind: EventKind.deletion,
        tags: const [
          ['h', _channelId],
          ['e', 'pending'],
        ],
        content: '',
        sig: 'sig',
      );
      final session = _RecordingRelaySessionNotifier(
        queryResults: [
          [_event(id: 'root', createdAt: 10), _bounds()],
          <NostrEvent>[],
          <NostrEvent>[],
          <NostrEvent>[],
          [deletion],
        ],
      );
      final container = _buildContainer(session);
      addTearDown(container.dispose);
      container.listen(channelMessagesProvider(_channelId), (_, _) {});
      await _pumpEventQueue();
      final notifier = container.read(
        channelMessagesProvider(_channelId).notifier,
      );
      notifier.addLocalMessage(
        _event(
          id: 'pending',
          createdAt: 20,
          extraTags: const [
            ['e', 'root', '', 'reply'],
          ],
        ),
      );
      const args = ThreadRepliesArgs(channelId: _channelId, rootId: 'root');
      container.listen(threadRepliesProvider(args), (_, _) {});
      await container.read(threadRepliesProvider(args).future);
      expect(
        container.read(threadLocalRepliesProvider(args)).single.id,
        'pending',
      );
      container.invalidate(threadRepliesProvider(args));
      await container.read(threadRepliesProvider(args).future);
      await _pumpEventQueue();
      expect(container.exists(threadLocalRepliesProvider(args)), isFalse);
      expect(container.read(pendingLocalMessagesProvider(_channelId)), isEmpty);
      final filters = session.queryFilters.where(
        (filter) => filter.kinds.contains(EventKind.deletion),
      );
      expect(filters, hasLength(2));
      expect(
        filters.every(
          (filter) =>
              filter.limit == 1 && filter.tags['#e']!.single == 'pending',
        ),
        isTrue,
      );
    },
  );

  test('a reply newer than the relay recount raises the badge', () async {
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

    relaySession.emit(
      _event(
        id: 'reply-1',
        createdAt: 20,
        extraTags: const [
          ['e', 'root', '', 'reply'],
        ],
      ),
    );
    relaySession.emit(_summary(rootId: 'root', replyCount: 1, createdAt: 20));
    // A second reply lands, and its recount is lost or still in flight.
    relaySession.emit(
      _event(
        id: 'reply-2',
        createdAt: 21,
        extraTags: const [
          ['e', 'root', '', 'reply'],
        ],
      ),
    );
    await _pumpEventQueue();

    final notifier = container.read(
      channelMessagesProvider(_channelId).notifier,
    );
    expect(notifier.threadSummaries['root']?.replyCount, 1);
    final entries = buildMainTimelineEntries(
      formatTimeline(
        container.read(channelMessagesProvider(_channelId)).value!,
      ),
      relaySummaries: notifier.threadSummaries,
    );
    expect(entries.single.message.id, 'root');
    expect(entries.single.summary!.replyCount, 2);
    expect(entries.single.summary!.lastReplyAt, 21);
  });

  test(
    'legacy pagination preserves desktop equal-second channel order',
    () async {
      final relaySession = _RecordingRelaySessionNotifier(
        historyResults: [
          [_event(id: 'a-head', createdAt: 20)],
          [
            _event(id: 'm-older', createdAt: 20),
            _event(id: 'z-older', createdAt: 20),
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
      await expectLater(notifier.fetchOlder(), completion(isTrue));

      expect(
        container
            .read(channelMessagesProvider(_channelId))
            .value
            ?.map((event) => event.id),
        ['z-older', 'm-older', 'a-head'],
      );
    },
  );

  test(
    'window pagination preserves desktop equal-second channel order',
    () async {
      final relaySession = _RecordingRelaySessionNotifier(
        queryResults: [
          [
            _event(id: 'a-head', createdAt: 20),
            _bounds(hasMore: true, cursorCreatedAt: 20, cursorId: 'a-head'),
          ],
          [
            _event(id: 'm-older', createdAt: 20),
            _event(id: 'z-older', createdAt: 20),
            _bounds(dTag: '${_channelId.toLowerCase()}:20:a-head'),
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
      await expectLater(notifier.fetchOlder(), completion(isTrue));

      expect(
        container
            .read(channelMessagesProvider(_channelId))
            .value
            ?.map((event) => event.id),
        ['z-older', 'm-older', 'a-head'],
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

class _IdReadTrackingEvent extends NostrEvent {
  final void Function() onIdRead;

  _IdReadTrackingEvent(NostrEvent event, {required this.onIdRead})
    : super(
        id: event.id,
        pubkey: event.pubkey,
        createdAt: event.createdAt,
        kind: event.kind,
        tags: event.tags,
        content: event.content,
        sig: event.sig,
      );

  @override
  String get id {
    onIdRead();
    return super.id;
  }
}

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

NostrEvent _huddleEvent({
  required String id,
  required int kind,
  required int createdAt,
}) {
  return NostrEvent(
    id: id,
    pubkey: 'alice',
    createdAt: createdAt,
    kind: kind,
    tags: const [
      ['h', _channelId],
    ],
    content: jsonEncode({
      'ephemeral_channel_id': '22222222-2222-4222-8222-222222222222',
    }),
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
  final Queue<List<NostrEvent>> _historyResults;
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
    List<List<NostrEvent>> historyResults = const [],
  }) : _queryResults = Queue<Object>.of(queryResults),
       _historyResults = Queue<List<NostrEvent>>.of(historyResults);

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
    if (_historyResults.isNotEmpty) {
      return Future.value(_historyResults.removeFirst());
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
