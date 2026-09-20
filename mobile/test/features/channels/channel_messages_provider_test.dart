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

  test(
    'scan preserves a reply acknowledged after query start without an echo',
    () async {
      final scan = Completer<List<NostrEvent>>();
      final session = _RecordingRelaySessionNotifier(
        queryResults: [
          [_event(id: 'root', createdAt: 10), _bounds()],
          scan.future,
          <NostrEvent>[],
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
          id: 'accepted-late',
          createdAt: 20,
          extraTags: const [
            ['e', 'root', '', 'reply'],
          ],
        ),
      );
      const args = ThreadRepliesArgs(channelId: _channelId, rootId: 'root');
      container.listen(threadRepliesProvider(args), (_, _) {});
      final result = container.read(threadRepliesProvider(args).future);
      await _pumpEventQueue();
      notifier.completeLocalMessage('accepted-late');
      scan.complete([]);
      await result;
      expect(notifier.cachedThreadReplyIds('root'), contains('accepted-late'));
      final entries = buildMainTimelineEntries(
        formatTimeline(
          container.read(channelMessagesProvider(_channelId)).value!,
        ),
        relaySummaries: notifier.threadSummaries,
      );
      expect(entries.single.summary!.replyCount, 1);
    },
  );

  test(
    'thread scan bounds pending deletion proofs and retains excess replies',
    () async {
      NostrEvent marker(int i) => NostrEvent(
        id: 'deletion-$i',
        pubkey: 'author',
        createdAt: 100,
        kind: EventKind.deletion,
        tags: [
          ['h', _channelId],
          ['e', 'pending-$i'],
        ],
        content: '',
        sig: '',
      );
      final session = _RecordingRelaySessionNotifier(
        queryResults: [
          [_event(id: 'root', createdAt: 10), _bounds()],
          <NostrEvent>[],
          [for (var i = 0; i < 20; i++) marker(i)],
          <NostrEvent>[],
          [for (var i = 20; i < 25; i++) marker(i)],
        ],
      );
      final container = _buildContainer(session);
      addTearDown(container.dispose);
      container.listen(channelMessagesProvider(_channelId), (_, _) {});
      await _pumpEventQueue();
      final notifier = container.read(
        channelMessagesProvider(_channelId).notifier,
      );
      for (var i = 0; i < 25; i++) {
        notifier.addLocalMessage(
          _event(
            id: 'pending-$i',
            createdAt: 20 + i,
            extraTags: const [
              ['e', 'root', '', 'reply'],
            ],
          ),
        );
      }
      const args = ThreadRepliesArgs(channelId: _channelId, rootId: 'root');
      container.listen(threadRepliesProvider(args), (_, _) {});
      await container.read(threadRepliesProvider(args).future);
      int proofs() => session.queryFilters
          .where((f) => f.kinds.contains(EventKind.deletion))
          .length;
      expect(proofs(), 20);
      expect(notifier.unconfirmedThreadReplyIds('root'), {
        for (var i = 20; i < 25; i++) 'pending-$i',
      });
      expect(container.read(threadLocalRepliesProvider(args)), hasLength(5));
      container.invalidate(threadRepliesProvider(args));
      await container.read(threadRepliesProvider(args).future);
      expect(proofs(), 25);
      expect(notifier.unconfirmedThreadReplyIds('root'), isEmpty);
      expect(container.read(threadLocalRepliesProvider(args)), isEmpty);
    },
  );

  for (final count in [1, 300]) {
    for (final acknowledgeDuringQuery in [false, true]) {
      test(
        'proof tombstone preserves $count surviving replies (late ACK: $acknowledgeDuringQuery)',
        () async {
          final scan = Completer<List<NostrEvent>>();
          final survivors = [
            for (var i = 0; i < count; i++)
              _event(
                id: 'survivor-$i',
                createdAt: 30 + i,
                extraTags: const [
                  ['e', 'root', '', 'reply'],
                ],
              ),
          ];
          final session = _RecordingRelaySessionNotifier(
            queryResults: [
              [_event(id: 'root', createdAt: 10), _bounds()],
              scan.future,
              if (count >= 200) survivors.skip(200).toList(),
              [
                NostrEvent(
                  id: 'offline-deletion',
                  pubkey: 'author',
                  createdAt: 25,
                  kind: EventKind.deletion,
                  tags: const [
                    ['h', _channelId],
                    ['e', 'deleted-local'],
                  ],
                  content: '',
                  sig: '',
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
          notifier.addLocalMessage(
            _event(
              id: 'deleted-local',
              createdAt: 20,
              extraTags: const [
                ['e', 'root', '', 'reply'],
              ],
            ),
          );
          const args = ThreadRepliesArgs(channelId: _channelId, rootId: 'root');
          container.listen(threadRepliesProvider(args), (_, _) {});
          final result = container.read(threadRepliesProvider(args).future);
          await _pumpEventQueue();
          if (acknowledgeDuringQuery) {
            notifier.completeLocalMessage('deleted-local');
          }
          scan.complete(survivors.take(200).toList());
          expect(await result, survivors);
          expect(notifier.threadSummaries['root']!.descendantCount, count);
          expect(notifier.unconfirmedThreadReplyIds('root'), isEmpty);
          final entries = buildMainTimelineEntries(
            formatTimeline(
              container.read(channelMessagesProvider(_channelId)).value!,
            ),
            relaySummaries: notifier.threadSummaries,
          );
          expect(entries.single.summary!.replyCount, count);
        },
      );
    }
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
              extraTags: [
                if (count == 600 && i > 0) ['e', 'root', '', 'root'],
                ['e', count == 600 && i > 0 ? 'reply-0' : 'root', '', 'reply'],
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
          queryResults: [window, ...pages, window, ...pages, ...pages],
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
        if (count == 600) {
          expect(
            notifier.cachedThreadReplyIds('root').contains('reply-0'),
            isFalse,
          );
        }
        session.setConnected(false);
        await _pumpEventQueue();
        session.setConnected(true);
        await _pumpEventQueue();
        await Future<void>.delayed(const Duration(milliseconds: 300));
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

  test(
    '600 live replies coalesce into one exact recount before reopen',
    () async {
      final replies = [
        for (var i = 0; i < 600; i++)
          _event(
            id: 'live-$i',
            createdAt: 20 + i,
            extraTags: const [
              ['e', 'root', '', 'reply'],
            ],
          ),
      ];
      final pages = [
        replies.take(200).toList(),
        replies.skip(200).take(200).toList(),
        replies.skip(400).toList(),
        <NostrEvent>[],
      ];
      final window = [_event(id: 'root', createdAt: 10), _bounds()];
      final session = _RecordingRelaySessionNotifier(
        queryResults: [window, ...pages, window, ...pages, ...pages],
      );
      final container = _buildContainer(session);
      addTearDown(container.dispose);
      container.listen(channelMessagesProvider(_channelId), (_, _) {});
      await _pumpEventQueue();
      final notifier = container.read(
        channelMessagesProvider(_channelId).notifier,
      );
      for (final reply in replies) {
        session.emit(reply);
      }
      expect(notifier.threadSummaries['root']?.isLowerBound, isTrue);
      expect(
        session.queryFilters.where(
          (filter) => filter.extensions.containsKey('depth_limit'),
        ),
        isEmpty,
      );
      await Future<void>.delayed(const Duration(milliseconds: 300));
      expect(notifier.threadSummaries['root']?.descendantCount, 600);
      expect(notifier.threadSummaries['root']?.isLowerBound, isFalse);
      expect(notifier.cachedThreadReplyIds('root'), hasLength(256));
      expect(
        session.queryFilters.where(
          (filter) => filter.extensions.containsKey('depth_limit'),
        ),
        hasLength(4),
      );
      const args = ThreadRepliesArgs(channelId: _channelId, rootId: 'root');
      expect(container.exists(threadRepliesProvider(args)), isFalse);
      session.setConnected(false);
      await _pumpEventQueue();
      session.setConnected(true);
      await _pumpEventQueue();
      await Future<void>.delayed(const Duration(milliseconds: 300));
      session.emit(_event(id: 'unrelated', createdAt: 1000));
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
        600,
      );
      final reopened = container.listen(threadRepliesProvider(args), (_, _) {});
      expect(
        await container.read(threadRepliesProvider(args).future),
        hasLength(600),
      );
      expect(notifier.threadSummaries['root']?.isLowerBound, isFalse);
      reopened.close();
    },
  );

  test(
    'overflow refreshes run at most two scans and stop on disposal',
    () async {
      final gates = [for (var i = 0; i < 3; i++) Completer<List<NostrEvent>>()];
      final session = _RecordingRelaySessionNotifier(
        queryResults: [
          [
            for (var root = 2; root >= 0; root--)
              _event(id: 'root-$root', createdAt: 10 + root),
            _bounds(),
          ],
          ...gates.map((gate) => gate.future),
        ],
      );
      final container = _buildContainer(session);
      container.listen(channelMessagesProvider(_channelId), (_, _) {});
      await _pumpEventQueue();
      for (var root = 0; root < 3; root++) {
        for (var i = 0; i < 300; i++) {
          session.emit(
            _event(
              id: '$root-$i',
              createdAt: 20 + i,
              extraTags: [
                ['e', 'root-$root', '', 'reply'],
              ],
            ),
          );
        }
      }
      await Future<void>.delayed(const Duration(milliseconds: 300));
      expect(
        session.queryFilters.where(
          (filter) => filter.extensions.containsKey('depth_limit'),
        ),
        hasLength(2),
      );
      gates[0].complete([]);
      gates[1].complete([]);
      await Future<void>.delayed(const Duration(milliseconds: 300));
      expect(
        session.queryFilters.where(
          (filter) => filter.extensions.containsKey('depth_limit'),
        ),
        hasLength(3),
      );
      container.dispose();
      gates[2].complete([]);
      await _pumpEventQueue();
      expect(
        session.queryFilters.where(
          (filter) => filter.extensions.containsKey('depth_limit'),
        ),
        hasLength(3),
      );
    },
  );

  test(
    'reconnect supersedes an in-flight overflow recount and resumes it',
    () async {
      final oldScan = Completer<List<NostrEvent>>();
      final replies = [
        for (var i = 0; i < 300; i++)
          _event(
            id: 'reply-$i',
            createdAt: 20 + i,
            extraTags: const [
              ['e', 'root', '', 'reply'],
            ],
          ),
      ];
      final window = [_event(id: 'root', createdAt: 10), _bounds()];
      final session = _RecordingRelaySessionNotifier(
        queryResults: [
          window,
          oldScan.future,
          window,
          replies.take(200).toList(),
          replies.skip(200).toList(),
        ],
      );
      final container = _buildContainer(session);
      addTearDown(container.dispose);
      container.listen(channelMessagesProvider(_channelId), (_, _) {});
      await _pumpEventQueue();
      final notifier = container.read(
        channelMessagesProvider(_channelId).notifier,
      );
      for (final reply in replies) {
        session.emit(reply);
      }
      await Future<void>.delayed(const Duration(milliseconds: 300));
      session.setConnected(false);
      await _pumpEventQueue();
      session.setConnected(true);
      await _pumpEventQueue();
      oldScan.complete(replies.take(200).toList());
      await Future<void>.delayed(const Duration(milliseconds: 300));
      expect(notifier.threadSummaries['root']?.descendantCount, 300);
      expect(notifier.threadSummaries['root']?.isLowerBound, isFalse);
      expect(
        session.queryFilters.where(
          (filter) => filter.extensions.containsKey('depth_limit'),
        ),
        hasLength(3),
      );
    },
  );

  test('exhausted overflow recount resumes on new activity', () async {
    final replies = [
      for (var i = 0; i < 301; i++)
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
        for (var i = 0; i < 3; i++) Exception('recount unavailable'),
        replies.take(200).toList(),
        replies.skip(200).toList(),
      ],
    );
    final container = _buildContainer(session);
    addTearDown(container.dispose);
    container.listen(channelMessagesProvider(_channelId), (_, _) {});
    await _pumpEventQueue();
    final notifier = container.read(
      channelMessagesProvider(_channelId).notifier,
    );
    for (final reply in replies.take(300)) {
      session.emit(reply);
    }
    await Future<void>.delayed(const Duration(milliseconds: 2000));
    expect(
      session.queryFilters.where(
        (filter) => filter.extensions.containsKey('depth_limit'),
      ),
      hasLength(3),
    );
    expect(notifier.threadSummaries['root']?.isLowerBound, isTrue);
    session.emit(replies.last);
    await Future<void>.delayed(const Duration(milliseconds: 300));
    expect(notifier.threadSummaries['root']?.descendantCount, 301);
    expect(notifier.threadSummaries['root']?.isLowerBound, isFalse);
  });

  test('a fresh relay summary cancels the queued overflow scan', () async {
    final session = _RecordingRelaySessionNotifier(
      queryResults: [
        [_event(id: 'root', createdAt: 10), _bounds()],
      ],
    );
    final container = _buildContainer(session);
    addTearDown(container.dispose);
    container.listen(channelMessagesProvider(_channelId), (_, _) {});
    await _pumpEventQueue();
    for (var i = 0; i < 300; i++) {
      session.emit(
        _event(
          id: 'reply-$i',
          createdAt: 20 + i,
          extraTags: const [
            ['e', 'root', '', 'reply'],
          ],
        ),
      );
    }
    session.emit(_summary(rootId: 'root', replyCount: 300, createdAt: 400));
    await Future<void>.delayed(const Duration(milliseconds: 300));
    final summary = container
        .read(channelMessagesProvider(_channelId).notifier)
        .threadSummaries['root'];
    expect(summary?.descendantCount, 300);
    expect(summary?.isLowerBound, isFalse);
    expect(
      session.queryFilters.where(
        (filter) => filter.extensions.containsKey('depth_limit'),
      ),
      isEmpty,
    );
  });

  test('overflow recounts pause while the channel has no listeners', () async {
    final replies = [
      for (var i = 0; i < 300; i++)
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
        replies.skip(200).toList(),
      ],
    );
    final container = _buildContainer(session);
    addTearDown(container.dispose);
    final subscription = container.listen(
      channelMessagesProvider(_channelId),
      (_, _) {},
    );
    await _pumpEventQueue();
    for (final reply in replies) {
      session.emit(reply);
    }
    subscription.close();
    await Future<void>.delayed(const Duration(milliseconds: 300));
    expect(
      session.queryFilters.where(
        (filter) => filter.extensions.containsKey('depth_limit'),
      ),
      isEmpty,
    );
    final resumed = container.listen(
      channelMessagesProvider(_channelId),
      (_, _) {},
    );
    await Future<void>.delayed(const Duration(milliseconds: 300));
    final summary = container
        .read(channelMessagesProvider(_channelId).notifier)
        .threadSummaries['root'];
    expect(summary?.descendantCount, 300);
    expect(summary?.isLowerBound, isFalse);
    resumed.close();
  });

  test('a newer route query wins over an older background recount', () async {
    final oldScan = Completer<List<NostrEvent>>();
    final replies = [
      for (var i = 0; i < 400; i++)
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
        oldScan.future,
        replies.take(200).toList(),
        replies.skip(200).toList(),
        <NostrEvent>[],
      ],
    );
    final container = _buildContainer(session);
    addTearDown(container.dispose);
    container.listen(channelMessagesProvider(_channelId), (_, _) {});
    await _pumpEventQueue();
    for (final reply in replies.take(300)) {
      session.emit(reply);
    }
    await Future<void>.delayed(const Duration(milliseconds: 300));
    const args = ThreadRepliesArgs(channelId: _channelId, rootId: 'root');
    container.listen(threadRepliesProvider(args), (_, _) {});
    expect(
      await container.read(threadRepliesProvider(args).future),
      hasLength(400),
    );
    oldScan.complete([]);
    await _pumpEventQueue();
    final summary = container
        .read(channelMessagesProvider(_channelId).notifier)
        .threadSummaries['root'];
    expect(summary?.descendantCount, 400);
    expect(summary?.isLowerBound, isFalse);
  });

  for (final fetched in [false, true]) {
    for (final count in [1, 257, 600]) {
      test(
        '$fetched deletion recounts a queried $count-reply aggregate',
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
          final remaining = replies.skip(1).toList();
          final session = _RecordingRelaySessionNotifier(
            queryResults: [
              [_event(id: 'root', createdAt: 10), _bounds()],
              for (var i = 0; i < remaining.length; i += 200)
                remaining.skip(i).take(200).toList(),
              if (remaining.length % 200 == 0) <NostrEvent>[],
            ],
          );
          final container = _buildContainer(session);
          addTearDown(container.dispose);
          container.listen(channelMessagesProvider(_channelId), (_, _) {});
          await _pumpEventQueue();
          final notifier = container.read(
            channelMessagesProvider(_channelId).notifier,
          );
          notifier.cacheCompleteThreadQuery('root', {}, replies);
          expect(notifier.threadSummaries['root']?.descendantCount, count);
          if (count == 600) {
            expect(
              notifier.cachedThreadReplyIds('root'),
              isNot(contains('reply-0')),
            );
          }
          final deletion = NostrEvent(
            id: 'delete-first',
            pubkey: 'author',
            createdAt: 1000,
            kind: EventKind.deletion,
            tags: const [
              ['h', _channelId],
              ['e', 'reply-0'],
            ],
            content: '',
            sig: '',
          );
          if (fetched) {
            notifier.cacheThreadDeletions([deletion]);
          } else {
            session.emit(deletion);
          }
          expect(notifier.threadSummaries['root']?.descendantCount, count - 1);
          expect(notifier.threadSummaries['root']?.isCountPending, isFalse);
          expect(notifier.threadSummaries['root']?.isLowerBound, isTrue);
          await Future<void>.delayed(const Duration(milliseconds: 300));
          expect(notifier.threadSummaries['root']?.descendantCount, count - 1);
          expect(notifier.threadSummaries['root']?.isLowerBound, isFalse);
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
            count == 1 ? null : count - 1,
          );
        },
      );
    }
  }

  for (final refreshedCount in [-1, 0, 300, 700]) {
    test(
      'reconnect summary replaces queried count with $refreshedCount',
      () async {
        final root = _event(id: 'root', createdAt: 10);
        final session = _RecordingRelaySessionNotifier(
          queryResults: [
            [root, _bounds()],
            [
              root,
              if (refreshedCount >= 0)
                _summary(
                  rootId: 'root',
                  replyCount: refreshedCount,
                  createdAt: 1000,
                ),
              _bounds(),
            ],
            if (refreshedCount <= 0) <NostrEvent>[],
          ],
        );
        final container = _buildContainer(session);
        addTearDown(container.dispose);
        container.listen(channelMessagesProvider(_channelId), (_, _) {});
        await _pumpEventQueue();
        final notifier = container.read(
          channelMessagesProvider(_channelId).notifier,
        );
        notifier.cacheCompleteThreadQuery('root', {}, [
          for (var i = 0; i < 600; i++)
            _event(
              id: 'reply-$i',
              createdAt: 20 + i,
              extraTags: const [
                ['e', 'root', '', 'reply'],
              ],
            ),
        ]);
        session.setConnected(false);
        await _pumpEventQueue();
        session.setConnected(true);
        await _pumpEventQueue();
        await Future<void>.delayed(const Duration(milliseconds: 300));
        expect(
          notifier.threadSummaries['root']?.descendantCount,
          refreshedCount < 0 ? 0 : refreshedCount,
        );
        expect(notifier.threadSummaries['root']?.isLowerBound, isFalse);
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
          refreshedCount <= 0 ? null : refreshedCount,
        );
      },
    );
  }

  test(
    'unknown deletion preserves unrelated navigation after recount failure',
    () async {
      final session = _RecordingRelaySessionNotifier(
        queryResults: [
          [
            _event(id: 'unrelated', createdAt: 10),
            _summary(rootId: 'unrelated', replyCount: 1),
            _bounds(),
          ],
          Exception('recount unavailable'),
        ],
      );
      final container = _buildContainer(session);
      addTearDown(container.dispose);
      container.listen(channelMessagesProvider(_channelId), (_, _) {});
      await _pumpEventQueue();
      session.emit(
        NostrEvent(
          id: 'deletion',
          pubkey: 'author',
          createdAt: 1000,
          kind: EventKind.deletion,
          tags: const [
            ['h', _channelId],
            ['e', 'unknown-old-reply'],
          ],
          content: '',
          sig: '',
        ),
      );
      await Future<void>.delayed(const Duration(milliseconds: 300));
      final notifier = container.read(
        channelMessagesProvider(_channelId).notifier,
      );
      final summary = notifier.threadSummaries['unrelated'];
      expect(summary?.descendantCount, 1);
      expect(summary?.isCountPending, isTrue);
      final entries = buildMainTimelineEntries(
        formatTimeline(
          container.read(channelMessagesProvider(_channelId)).value!,
        ),
        relaySummaries: notifier.threadSummaries,
      );
      expect(entries.single.summary?.replyCount, 1);
      expect(entries.single.summary?.isCountPending, isTrue);
    },
  );

  for (final olderCount in [0, 3]) {
    test(
      'pagination reconciles pinned-root query totals to $olderCount',
      () async {
        final session = _RecordingRelaySessionNotifier(
          queryResults: [
            [
              _event(id: 'newest', createdAt: 100),
              _bounds(hasMore: true, cursorCreatedAt: 100, cursorId: 'newest'),
            ],
            [
              _event(id: 'old-root', createdAt: 10),
              if (olderCount > 0)
                _summary(rootId: 'old-root', replyCount: olderCount),
              _bounds(dTag: '${_channelId.toLowerCase()}:100:newest'),
            ],
            if (olderCount == 0) <NostrEvent>[],
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
        notifier.cacheCompleteThreadQuery('old-root', {}, [
          _event(
            id: 'old-reply',
            createdAt: 20,
            extraTags: const [
              ['e', 'old-root', '', 'reply'],
            ],
          ),
        ]);
        expect(notifier.threadSummaries['old-root']?.descendantCount, 1);
        expect(await notifier.fetchOlder(), isTrue);
        await Future<void>.delayed(const Duration(milliseconds: 300));
        expect(
          notifier.threadSummaries['old-root']?.descendantCount,
          olderCount,
        );
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
          olderCount == 0 ? null : olderCount,
        );
      },
    );
  }

  test(
    'continued replies do not restart an active paginated recount',
    () async {
      final replies = [
        for (var i = 0; i < 603; i++)
          _event(
            id: 'reply-$i',
            createdAt: 20 + i,
            extraTags: const [
              ['e', 'root', '', 'reply'],
            ],
          ),
      ];
      final gates = [for (var i = 0; i < 4; i++) Completer<List<NostrEvent>>()];
      final session = _RecordingRelaySessionNotifier(
        queryResults: [
          [_event(id: 'root', createdAt: 10), _bounds()],
          for (final gate in gates) gate.future,
          for (var i = 0; i < 603; i += 200) replies.skip(i).take(200).toList(),
        ],
      );
      final container = _buildContainer(session);
      addTearDown(container.dispose);
      container.listen(channelMessagesProvider(_channelId), (_, _) {});
      await _pumpEventQueue();
      for (final reply in replies.take(300)) {
        session.emit(reply);
      }
      await Future<void>.delayed(const Duration(milliseconds: 300));
      for (var page = 0; page < 3; page++) {
        session.emit(replies[600 + page]);
        gates[page].complete(replies.skip(page * 200).take(200).toList());
        await _pumpEventQueue();
        final scans = session.queryFilters
            .where((filter) => filter.extensions.containsKey('depth_limit'))
            .toList();
        expect(scans, hasLength(page + 2));
        expect(
          scans.last.extensions['thread_cursor'],
          replies[(page + 1) * 200 - 1].createdAt,
        );
      }
      gates.last.complete([]);
      await _pumpEventQueue();
      final notifier = container.read(
        channelMessagesProvider(_channelId).notifier,
      );
      expect(notifier.threadSummaries['root']?.descendantCount, 603);
      expect(notifier.threadSummaries['root']?.isLowerBound, isTrue);
      await Future<void>.delayed(const Duration(milliseconds: 300));
      expect(notifier.threadSummaries['root']?.descendantCount, 603);
      expect(notifier.threadSummaries['root']?.isLowerBound, isFalse);
      expect(
        session.queryFilters.where(
          (filter) => filter.extensions.containsKey('depth_limit'),
        ),
        hasLength(8),
      );
    },
  );

  for (final newest in [false, true]) {
    test(
      'page launched before route query cannot supersede it (newest: $newest)',
      () async {
        final page = Completer<List<NostrEvent>>();
        final scan = Completer<List<NostrEvent>>();
        final session = _RecordingRelaySessionNotifier(
          queryResults: [
            [
              _event(id: 'newest', createdAt: 100),
              _bounds(hasMore: true, cursorCreatedAt: 100, cursorId: 'newest'),
            ],
            page.future,
            scan.future,
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
        Future<bool>? older;
        if (newest) {
          session.setConnected(false);
          await _pumpEventQueue();
          session.setConnected(true);
          await _pumpEventQueue();
        } else {
          older = notifier.fetchOlder();
        }
        const args = ThreadRepliesArgs(
          channelId: _channelId,
          rootId: 'old-root',
        );
        container.listen(threadRepliesProvider(args), (_, _) {});
        final thread = container.read(threadRepliesProvider(args).future);
        await _pumpEventQueue();
        page.complete([
          _event(id: 'old-root', createdAt: 10),
          _bounds(
            dTag: newest
                ? '${_channelId.toLowerCase()}:head'
                : '${_channelId.toLowerCase()}:100:newest',
          ),
        ]);
        if (older != null) await older;
        await _pumpEventQueue();
        scan.complete([
          _event(
            id: 'reply',
            createdAt: 20,
            extraTags: const [
              ['e', 'old-root', '', 'reply'],
            ],
          ),
        ]);
        await thread;
        expect(
          session.historyFilters.where((filter) => filter.ids == null),
          isEmpty,
        );
        expect(notifier.threadSummaries['old-root']?.descendantCount, 1);
      },
    );
  }

  test(
    'unknown deletion performs one ownership lookup without recount fan-out',
    () async {
      final session = _RecordingRelaySessionNotifier(
        queryResults: [
          [
            for (var i = 39; i >= 0; i--) ...[
              _event(id: 'root-$i', createdAt: 10 + i),
              _summary(rootId: 'root-$i', replyCount: 1),
            ],
            _bounds(),
          ],
          <NostrEvent>[],
        ],
      );
      final container = _buildContainer(session);
      addTearDown(container.dispose);
      container.listen(channelMessagesProvider(_channelId), (_, _) {});
      await _pumpEventQueue();
      for (final kind in [EventKind.deletion, EventKind.nip29DeleteEvent]) {
        session.emit(
          NostrEvent(
            id: 'delete-$kind',
            pubkey: 'author',
            createdAt: 100,
            kind: kind,
            tags: const [
              ['h', _channelId],
              ['e', 'unknown'],
            ],
            content: '',
            sig: '',
          ),
        );
      }
      await Future<void>.delayed(const Duration(milliseconds: 300));
      expect(
        session.queryFilters.where(
          (filter) => filter.ids?.contains('unknown') ?? false,
        ),
        hasLength(1),
      );
      expect(
        session.queryFilters.where(
          (filter) => filter.extensions.containsKey('depth_limit'),
        ),
        isEmpty,
      );
      final summaries = container
          .read(channelMessagesProvider(_channelId).notifier)
          .threadSummaries;
      expect(summaries, hasLength(40));
      expect(
        summaries.values.every(
          (summary) => summary.descendantCount == 1 && summary.isCountPending,
        ),
        isTrue,
      );
    },
  );

  test(
    'known deletion fences an older ownership response before recount',
    () async {
      final ownership = Completer<List<NostrEvent>>();
      final session = _RecordingRelaySessionNotifier(
        queryResults: [
          [_event(id: 'root', createdAt: 10), _bounds()],
          ownership.future,
          <NostrEvent>[],
        ],
      );
      final container = _buildContainer(session);
      addTearDown(container.dispose);
      container.listen(channelMessagesProvider(_channelId), (_, _) {});
      await _pumpEventQueue();
      final notifier = container.read(
        channelMessagesProvider(_channelId).notifier,
      );
      notifier.cacheCompleteThreadQuery('root', {}, [
        _event(
          id: 'known',
          createdAt: 20,
          extraTags: const [
            ['e', 'root', '', 'reply'],
          ],
        ),
      ]);
      for (final target in ['unknown', 'known']) {
        session.emit(
          NostrEvent(
            id: 'delete-$target',
            pubkey: 'author',
            createdAt: 100,
            kind: EventKind.deletion,
            tags: [
              ['h', _channelId],
              ['e', target],
            ],
            content: '',
            sig: '',
          ),
        );
      }
      ownership.complete([_summary(rootId: 'root', replyCount: 1)]);
      await _pumpEventQueue();
      expect(notifier.threadSummaries['root']!.descendantCount, 0);
      await Future<void>.delayed(const Duration(milliseconds: 300));
      expect(
        session.queryFilters.where(
          (f) => f.extensions.containsKey('depth_limit'),
        ),
        hasLength(1),
      );
      final entries = buildMainTimelineEntries(
        formatTimeline(
          container.read(channelMessagesProvider(_channelId)).value!,
        ),
        relaySummaries: notifier.threadSummaries,
      );
      expect(
        entries.singleWhere((e) => e.message.id == 'root').summary,
        isNull,
      );
    },
  );

  for (final source in ['overflow', 'fallback']) {
    for (final outcome in ['recovered', 'exhausted', 'disposed']) {
      test('thread recount retry $source: $outcome', () async {
        final replies = [
          for (var i = 0; i < (source == 'overflow' ? 257 : 1); i++)
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
            if (source == 'fallback') Exception('NIP-CW unavailable'),
            Exception('temporary recount outage'),
            if (outcome == 'recovered') ...[
              if (source == 'overflow') ...[
                replies.take(200).toList(),
                replies.skip(200).toList(),
              ] else
                <NostrEvent>[],
            ] else ...[
              Exception('still unavailable'),
              Exception('still unavailable'),
            ],
          ],
          historyResults: [
            [_event(id: 'root', createdAt: 10)],
          ],
        );
        final container = _buildContainer(session);
        if (outcome != 'disposed') addTearDown(container.dispose);
        container.listen(channelMessagesProvider(_channelId), (_, _) {});
        await _pumpEventQueue();
        final notifier = container.read(
          channelMessagesProvider(_channelId).notifier,
        );
        for (final reply in replies) {
          session.emit(reply);
        }
        if (source == 'fallback') {
          session.setConnected(false);
          await _pumpEventQueue();
          session.setConnected(true);
        }
        await Future<void>.delayed(const Duration(milliseconds: 300));
        int scans() => session.queryFilters
            .where((f) => f.extensions.containsKey('depth_limit'))
            .length;
        expect(scans(), 1);
        if (outcome == 'disposed') container.dispose();
        await Future<void>.delayed(
          Duration(milliseconds: outcome == 'exhausted' ? 3800 : 800),
        );
        expect(
          scans(),
          outcome == 'disposed'
              ? 1
              : outcome == 'exhausted' || source == 'overflow'
              ? 3
              : 2,
        );
        if (outcome == 'disposed') return;
        final summary = notifier.threadSummaries['root']!;
        if (outcome == 'exhausted') {
          expect(summary.isLowerBound, isTrue);
        } else {
          expect(summary.isLowerBound, isFalse);
          expect(summary.isCountPending, isFalse);
          expect(summary.descendantCount, source == 'overflow' ? 257 : 0);
          if (source == 'fallback') {
            final entries = buildMainTimelineEntries(
              formatTimeline(
                container.read(channelMessagesProvider(_channelId)).value!,
              ),
              relaySummaries: notifier.threadSummaries,
            );
            expect(entries.single.summary, isNull);
          }
        }
      });
    }
  }

  for (final failureFirst in [false, true]) {
    test(
      'failed route query does not fence successful page (failure first: $failureFirst)',
      () async {
        final page = Completer<List<NostrEvent>>();
        final route = Completer<List<NostrEvent>>();
        final session = _RecordingRelaySessionNotifier(
          queryResults: [
            [_event(id: 'root', createdAt: 10), _bounds()],
            page.future,
            route.future,
            <NostrEvent>[],
          ],
        );
        final container = ProviderContainer(
          retry: (_, _) => null,
          overrides: [relaySessionProvider.overrideWith(() => session)],
        );
        addTearDown(container.dispose);
        container.listen(channelMessagesProvider(_channelId), (_, _) {});
        await _pumpEventQueue();
        final notifier = container.read(
          channelMessagesProvider(_channelId).notifier,
        );
        notifier.cacheCompleteThreadQuery('root', {}, [
          _event(
            id: 'deleted-offline',
            createdAt: 20,
            extraTags: const [
              ['e', 'root', '', 'reply'],
            ],
          ),
        ]);
        session.setConnected(false);
        await _pumpEventQueue();
        session.setConnected(true);
        await _pumpEventQueue();
        const args = ThreadRepliesArgs(channelId: _channelId, rootId: 'root');
        container.listen(threadRepliesProvider(args), (_, _) {});
        final failure = expectLater(
          container.read(threadRepliesProvider(args).future),
          throwsException,
        );
        await _pumpEventQueue();
        if (failureFirst) {
          route.completeError(Exception('route query failed'));
          await failure;
        }
        page.complete([_event(id: 'root', createdAt: 10), _bounds()]);
        await _pumpEventQueue();
        if (!failureFirst) {
          route.completeError(Exception('route query failed'));
          await failure;
        }
        await Future<void>.delayed(const Duration(milliseconds: 300));
        final entries = buildMainTimelineEntries(
          formatTimeline(
            container.read(channelMessagesProvider(_channelId)).value!,
          ),
          relaySummaries: notifier.threadSummaries,
        );
        expect(entries.single.summary, isNull);
        expect(notifier.cachedThreadReplyIds('root'), isEmpty);
      },
    );
  }

  for (final phase in ['queued', 'inflight', 'late', 'backoff', 'exhausted']) {
    test('ownership recovery resumes across reconnect: $phase', () async {
      final first = Completer<List<NostrEvent>>();
      final second = Completer<List<NostrEvent>>();
      final session = _RecordingRelaySessionNotifier(
        queryResults: [
          [
            _event(id: 'retained', createdAt: 10),
            _summary(rootId: 'retained', replyCount: 1),
            _bounds(),
          ],
          if (phase == 'backoff')
            Exception('temporary outage')
          else
            first.future,
          if (phase == 'queued') second.future,
          [_event(id: 'newest', createdAt: 200), _bounds()],
          if (phase == 'queued') ...[<NostrEvent>[], <NostrEvent>[]],
          if (phase == 'exhausted') ...[
            Exception('unavailable'),
            Exception('unavailable'),
            Exception('unavailable'),
          ] else
            [_summary(rootId: 'retained', replyCount: 0)],
          if (phase != 'exhausted') <NostrEvent>[],
        ],
      );
      final container = _buildContainer(session);
      addTearDown(container.dispose);
      container.listen(channelMessagesProvider(_channelId), (_, _) {});
      await _pumpEventQueue();
      final notifier = container.read(
        channelMessagesProvider(_channelId).notifier,
      );
      await notifier.loadEventsById(['retained']);
      notifier.cacheCompleteThreadQuery('retained', {}, [
        for (var i = 0; i < 257; i++)
          _event(
            id: 'old-reply-$i',
            createdAt: 20 + i,
            extraTags: const [
              ['e', 'retained', '', 'reply'],
            ],
          ),
      ]);

      for (var i = 0; i < (phase == 'queued' ? 3 : 1); i++) {
        session.emit(
          NostrEvent(
            id: 'delete-$i',
            pubkey: 'author',
            createdAt: 100 + i,
            kind: EventKind.deletion,
            tags: [
              ['h', _channelId],
              ['e', 'unknown-$i'],
            ],
            content: '',
            sig: '',
          ),
        );
      }
      await _pumpEventQueue();
      session.setConnected(false);
      await _pumpEventQueue();
      if (phase != 'backoff' && phase != 'late') first.complete([]);
      if (phase == 'queued') second.complete([]);
      await Future<void>.delayed(const Duration(milliseconds: 600));
      int requests() => session.queryFilters
          .where((f) => f.extensions['resolve_thread_roots'] == true)
          .length;
      expect(
        requests(),
        phase == 'queued' ? 2 : 1,
        reason: 'no ownership work runs while disconnected',
      );
      session.setConnected(true);
      await _pumpEventQueue();
      if (phase == 'late') first.complete([]);
      await Future<void>.delayed(
        Duration(milliseconds: phase == 'exhausted' ? 3800 : 400),
      );
      expect(
        requests(),
        phase == 'queued'
            ? 5
            : phase == 'exhausted'
            ? 4
            : 2,
      );
      final entries = buildMainTimelineEntries(
        formatTimeline(
          container.read(channelMessagesProvider(_channelId)).value!,
        ),
        relaySummaries: notifier.threadSummaries,
      );
      final retained = entries.singleWhere((e) => e.message.id == 'retained');
      if (phase == 'exhausted') {
        expect(retained.summary!.isCountPending, isTrue);
      } else {
        expect(
          retained.summary,
          isNull,
          reason: 'the off-window retained root is reconciled',
        );
      }
    });
  }

  for (final querySucceeds in [false, true]) {
    test(
      'skipped ownership response preserves uncertainty (query succeeds: $querySucceeds)',
      () async {
        final ownership = Completer<List<NostrEvent>>();
        final threadQuery = Completer<List<NostrEvent>>();
        final session = _RecordingRelaySessionNotifier(
          queryResults: [
            [
              _event(id: 'root', createdAt: 10),
              _summary(rootId: 'root', replyCount: 1),
              _bounds(),
            ],
            ownership.future,
            threadQuery.future,
          ],
        );
        final container = ProviderContainer(
          retry: (_, _) => null,
          overrides: [relaySessionProvider.overrideWith(() => session)],
        );
        addTearDown(container.dispose);
        container.listen(channelMessagesProvider(_channelId), (_, _) {});
        await _pumpEventQueue();
        final notifier = container.read(
          channelMessagesProvider(_channelId).notifier,
        );
        session.emit(
          NostrEvent(
            id: 'delete-unknown',
            pubkey: 'author',
            createdAt: 100,
            kind: EventKind.deletion,
            tags: const [
              ['h', _channelId],
              ['e', 'unknown'],
            ],
            content: '',
            sig: '',
          ),
        );
        const args = ThreadRepliesArgs(channelId: _channelId, rootId: 'root');
        container.listen(threadRepliesProvider(args), (_, _) {});
        final result = container.read(threadRepliesProvider(args).future);
        await _pumpEventQueue();
        ownership.complete([_summary(rootId: 'root', replyCount: 0)]);
        await _pumpEventQueue();
        expect(notifier.threadSummaries['root']!.isCountPending, isTrue);
        if (querySucceeds) {
          threadQuery.complete([]);
          await result;
        } else {
          final failure = expectLater(result, throwsException);
          threadQuery.completeError(Exception('newer query failed'));
          await failure;
        }
        await _pumpEventQueue();
        expect(
          notifier.threadSummaries['root']!.isCountPending,
          !querySucceeds,
        );
        final entries = buildMainTimelineEntries(
          formatTimeline(
            container.read(channelMessagesProvider(_channelId)).value!,
          ),
          relaySummaries: notifier.threadSummaries,
        );
        if (querySucceeds) {
          expect(entries.single.summary, isNull);
        } else {
          expect(entries.single.summary!.isCountPending, isTrue);
        }
      },
    );
  }

  for (final outcome in ['recovered', 'exhausted', 'disposed']) {
    test('deletion ownership retries are bounded: $outcome', () async {
      final session = _RecordingRelaySessionNotifier(
        queryResults: [
          [
            _event(id: 'root', createdAt: 10),
            _summary(rootId: 'root', replyCount: 1),
            _bounds(),
          ],
          Exception('temporary outage'),
          if (outcome == 'recovered')
            [_summary(rootId: 'root', replyCount: 0)]
          else ...[
            Exception('still unavailable'),
            Exception('still unavailable'),
          ],
        ],
      );
      final container = _buildContainer(session);
      if (outcome != 'disposed') addTearDown(container.dispose);
      container.listen(channelMessagesProvider(_channelId), (_, _) {});
      await _pumpEventQueue();
      final notifier = container.read(
        channelMessagesProvider(_channelId).notifier,
      );
      final deletion = NostrEvent(
        id: 'delete-unknown',
        pubkey: 'author',
        createdAt: 100,
        kind: EventKind.deletion,
        tags: const [
          ['h', _channelId],
          ['e', 'unknown'],
        ],
        content: '',
        sig: '',
      );
      session.emit(deletion);
      await _pumpEventQueue();
      int requests() => session.queryFilters
          .where((f) => f.extensions['resolve_thread_roots'] == true)
          .length;
      expect(requests(), 1);
      expect(notifier.threadSummaries['root']!.isCountPending, isTrue);
      // A replay is deduplicated, but must not prevent the scheduled recovery.
      session.emit(deletion);
      if (outcome == 'disposed') container.dispose();
      await Future<void>.delayed(
        Duration(milliseconds: outcome == 'exhausted' ? 3800 : 800),
      );
      expect(requests(), switch (outcome) {
        'recovered' => 2,
        'disposed' => 1,
        _ => 3,
      });
      if (outcome == 'recovered') {
        final entries = buildMainTimelineEntries(
          formatTimeline(
            container.read(channelMessagesProvider(_channelId)).value!,
          ),
          relaySummaries: notifier.threadSummaries,
        );
        expect(entries.single.summary, isNull);
        expect(notifier.threadSummaries['root']!.isCountPending, isFalse);
      } else if (outcome == 'exhausted') {
        expect(notifier.threadSummaries['root']!.isCountPending, isTrue);
      }
    });
  }

  for (final count in [150, 300]) {
    test(
      'large deletion event queues $count targets within shared bounds',
      () async {
        final first = Completer<List<NostrEvent>>();
        final second = Completer<List<NostrEvent>>();
        final session = _RecordingRelaySessionNotifier(
          queryResults: [
            [
              _event(id: 'root', createdAt: 10),
              _summary(rootId: 'root', replyCount: 1),
              _bounds(),
            ],
            first.future,
            second.future,
            for (var i = 2; i < count; i++)
              i == count - 1
                  ? [_summary(rootId: 'root', replyCount: 0)]
                  : <NostrEvent>[],
          ],
        );
        final container = _buildContainer(session);
        addTearDown(container.dispose);
        container.listen(channelMessagesProvider(_channelId), (_, _) {});
        await _pumpEventQueue();
        final notifier = container.read(
          channelMessagesProvider(_channelId).notifier,
        );
        final deletion = NostrEvent(
          id: 'large-delete',
          pubkey: 'author',
          createdAt: 100,
          kind: EventKind.deletion,
          tags: [
            ['h', _channelId],
            for (var i = 0; i < count; i++) ['e', 'unknown-$i'],
          ],
          content: '',
          sig: '',
        );
        session.emit(deletion);
        int requests() => session.queryFilters
            .where((f) => f.extensions['resolve_thread_roots'] == true)
            .length;
        await _pumpEventQueue();
        expect(requests(), 2);
        first.complete([]);
        second.complete([]);
        await _pumpEventQueue();
        expect(requests(), count);
        if (count == 150) {
          expect(notifier.threadSummaries['root']!.descendantCount, 0);
          final entries = buildMainTimelineEntries(
            formatTimeline(
              container.read(channelMessagesProvider(_channelId)).value!,
            ),
            relaySummaries: notifier.threadSummaries,
          );
          expect(entries.single.summary, isNull);
        } else {
          expect(notifier.threadSummaries['root']!.descendantCount, 0);
          expect(notifier.threadSummaries['root']!.isCountPending, isFalse);
          session.emit(deletion);
          await _pumpEventQueue();
          expect(
            requests(),
            count,
            reason: 'admitted targets stay deduplicated',
          );
        }
      },
    );
  }

  test('deferred ownership keeps its original evidence version', () async {
    final first = Completer<List<NostrEvent>>();
    final second = Completer<List<NostrEvent>>();
    final session = _RecordingRelaySessionNotifier(
      queryResults: [
        [
          _event(id: 'root', createdAt: 10),
          _summary(rootId: 'root', replyCount: 1),
          _bounds(),
        ],
        first.future,
        second.future,
        for (var i = 2; i < 258; i++) <NostrEvent>[],
        [_summary(rootId: 'root', replyCount: 0)],
      ],
    );
    final container = _buildContainer(session);
    addTearDown(container.dispose);
    container.listen(channelMessagesProvider(_channelId), (_, _) {});
    await _pumpEventQueue();
    session.emit(
      NostrEvent(
        id: 'large-delete',
        pubkey: 'author',
        createdAt: 100,
        kind: EventKind.deletion,
        tags: [
          ['h', _channelId],
          for (var i = 0; i < 259; i++) ['e', 'unknown-$i'],
        ],
        content: '',
        sig: '',
      ),
    );
    session.emit(_summary(rootId: 'root', replyCount: 1));
    first.complete([]);
    second.complete([]);
    await _pumpEventQueue();
    expect(
      session.queryFilters.where(
        (f) => f.extensions['resolve_thread_roots'] == true,
      ),
      hasLength(259),
    );
    final summary = container
        .read(channelMessagesProvider(_channelId).notifier)
        .threadSummaries['root']!;
    expect(summary.descendantCount, 1);
    expect(summary.isCountPending, isFalse);
  });

  for (final disposeEarly in [false, true]) {
    test(
      'ownership recovery bounds deletion bursts (dispose: $disposeEarly)',
      () async {
        final first = Completer<List<NostrEvent>>();
        final second = Completer<List<NostrEvent>>();
        final session = _RecordingRelaySessionNotifier(
          queryResults: [
            [
              _event(id: 'root', createdAt: 10),
              _summary(rootId: 'root', replyCount: 1),
              _bounds(),
            ],
            first.future,
            second.future,
            for (var i = 0; i < 298; i++) <NostrEvent>[],
          ],
        );
        final container = _buildContainer(session);
        if (!disposeEarly) addTearDown(container.dispose);
        container.listen(channelMessagesProvider(_channelId), (_, _) {});
        await _pumpEventQueue();
        final notifier = container.read(
          channelMessagesProvider(_channelId).notifier,
        );
        for (var i = 0; i < 300; i++) {
          session.emit(
            NostrEvent(
              id: 'delete-$i',
              pubkey: 'author',
              createdAt: 100 + i,
              kind: EventKind.deletion,
              tags: [
                ['h', _channelId],
                ['e', 'unknown-$i'],
              ],
              content: '',
              sig: '',
            ),
          );
        }
        int requests() => session.queryFilters
            .where((f) => f.extensions['resolve_thread_roots'] == true)
            .length;
        await _pumpEventQueue();
        expect(
          requests(),
          2,
          reason: 'requests share one concurrency budget across events',
        );
        if (disposeEarly) container.dispose();
        first.complete([]);
        second.complete([]);
        await _pumpEventQueue();
        expect(
          requests(),
          disposeEarly ? 2 : 300,
          reason: 'deferred targets drain only while mounted',
        );
        if (!disposeEarly) {
          expect(notifier.threadSummaries['root']!.isCountPending, isTrue);
          session.emit(_summary(rootId: 'root', replyCount: 1));
          expect(notifier.threadSummaries['root']!.isCountPending, isFalse);
        }
      },
    );
  }

  for (final newestFirst in [false, true]) {
    test(
      'overlapping deletion lookups preserve start order (newest first: $newestFirst)',
      () async {
        final first = Completer<List<NostrEvent>>();
        final second = Completer<List<NostrEvent>>();
        final session = _RecordingRelaySessionNotifier(
          queryResults: [
            [
              for (final id in ['owner', 'unrelated']) ...[
                _event(id: id, createdAt: 10),
                _summary(rootId: id, replyCount: 2),
              ],
              _bounds(),
            ],
            first.future,
            second.future,
          ],
        );
        final container = _buildContainer(session);
        addTearDown(container.dispose);
        container.listen(channelMessagesProvider(_channelId), (_, _) {});
        await _pumpEventQueue();
        final notifier = container.read(
          channelMessagesProvider(_channelId).notifier,
        );
        for (var i = 0; i < 2; i++) {
          session.emit(
            NostrEvent(
              id: 'delete-$i',
              pubkey: 'author',
              createdAt: 100 + i,
              kind: EventKind.deletion,
              tags: [
                ['h', _channelId],
                ['e', 'unknown-$i'],
              ],
              content: '',
              sig: '',
            ),
          );
        }
        await _pumpEventQueue();
        expect(notifier.threadSummaries['unrelated']!.isCountPending, isTrue);
        if (newestFirst) {
          second.complete([_summary(rootId: 'owner', replyCount: 0)]);
        } else {
          first.complete([_summary(rootId: 'owner', replyCount: 1)]);
        }
        await _pumpEventQueue();
        expect(notifier.threadSummaries['unrelated']!.isCountPending, isTrue);
        if (newestFirst) {
          first.complete([_summary(rootId: 'owner', replyCount: 1)]);
        } else {
          second.complete([_summary(rootId: 'owner', replyCount: 0)]);
        }
        await _pumpEventQueue();
        expect(notifier.threadSummaries['owner']!.descendantCount, 0);
        expect(notifier.threadSummaries['unrelated']!.descendantCount, 2);
        expect(notifier.threadSummaries['unrelated']!.isCountPending, isFalse);
        expect(notifier.threadSummaries['unrelated']!.isLowerBound, isFalse);
        expect(notifier.threadSummaries['owner']!.isCountPending, isFalse);
      },
    );
  }

  for (final secondResolves in [true, false]) {
    test(
      'multi-target deletion correlates shared roots (second resolves: $secondResolves)',
      () async {
        final first = Completer<List<NostrEvent>>();
        final second = Completer<List<NostrEvent>>();
        final session = _RecordingRelaySessionNotifier(
          queryResults: [
            [
              for (final id in ['owner', 'unrelated']) ...[
                _event(id: id, createdAt: 10),
                _summary(rootId: id, replyCount: 2),
              ],
              _bounds(),
            ],
            first.future,
            second.future,
          ],
        );
        final container = _buildContainer(session);
        addTearDown(container.dispose);
        container.listen(channelMessagesProvider(_channelId), (_, _) {});
        await _pumpEventQueue();
        final notifier = container.read(
          channelMessagesProvider(_channelId).notifier,
        );
        session.emit(
          NostrEvent(
            id: 'multi-delete',
            pubkey: 'author',
            createdAt: 100,
            kind: EventKind.nip29DeleteEvent,
            tags: const [
              ['h', _channelId],
              ['e', 'first-target'],
              ['e', 'second-target'],
            ],
            content: '',
            sig: '',
          ),
        );
        await _pumpEventQueue();
        first.complete([_summary(rootId: 'owner', replyCount: 1)]);
        await _pumpEventQueue();
        expect(
          notifier.threadSummaries['unrelated']!.isCountPending,
          isTrue,
          reason: 'the queued sibling still has unresolved ownership',
        );
        expect(notifier.threadSummaries['owner']!.isCountPending, isTrue);
        second.complete(
          secondResolves ? [_summary(rootId: 'owner', replyCount: 0)] : [],
        );
        await _pumpEventQueue();
        expect(notifier.threadSummaries['unrelated']!.descendantCount, 2);
        expect(
          notifier.threadSummaries['unrelated']!.isCountPending,
          !secondResolves,
        );
        expect(
          notifier.threadSummaries['owner']!.descendantCount,
          secondResolves ? 0 : 1,
        );
        final requests = session.queryFilters.where(
          (f) => f.extensions['resolve_thread_roots'] == true,
        );
        expect(requests.map((f) => f.ids!.single), [
          'first-target',
          'second-target',
        ]);
        final entries = buildMainTimelineEntries(
          formatTimeline(
            container.read(channelMessagesProvider(_channelId)).value!,
          ),
          relaySummaries: notifier.threadSummaries,
        );
        expect(
          entries.singleWhere((e) => e.message.id == 'owner').summary == null,
          secondResolves,
        );
      },
    );
  }

  test(
    'failed overlapping ownership lookup keeps uncertainty until fresh summary',
    () async {
      final first = Completer<List<NostrEvent>>();
      final second = Completer<List<NostrEvent>>();
      final session = _RecordingRelaySessionNotifier(
        queryResults: [
          [
            for (final id in ['owner', 'unrelated']) ...[
              _event(id: id, createdAt: 10),
              _summary(rootId: id, replyCount: 2),
            ],
            _bounds(),
          ],
          first.future,
          second.future,
        ],
      );
      final container = _buildContainer(session);
      addTearDown(container.dispose);
      container.listen(channelMessagesProvider(_channelId), (_, _) {});
      await _pumpEventQueue();
      final notifier = container.read(
        channelMessagesProvider(_channelId).notifier,
      );
      for (var i = 0; i < 2; i++) {
        session.emit(
          NostrEvent(
            id: 'delete-$i',
            pubkey: 'author',
            createdAt: 100 + i,
            kind: EventKind.deletion,
            tags: [
              ['h', _channelId],
              ['e', 'unknown-$i'],
            ],
            content: '',
            sig: '',
          ),
        );
      }
      first.completeError(Exception('ownership unavailable'));
      await _pumpEventQueue();
      second.complete([_summary(rootId: 'owner', replyCount: 0)]);
      await _pumpEventQueue();
      expect(notifier.threadSummaries['owner']!.descendantCount, 0);
      expect(notifier.threadSummaries['unrelated']!.isCountPending, isTrue);
      session.emit(_summary(rootId: 'unrelated', replyCount: 2));
      expect(notifier.threadSummaries['unrelated']!.isCountPending, isFalse);
      expect(notifier.threadSummaries['unrelated']!.isLowerBound, isFalse);
    },
  );

  test(
    'unscoped deletion evidence requires a matching queried target',
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
      for (final kind in [EventKind.deletion, EventKind.nip29DeleteEvent]) {
        final deletion = NostrEvent(
          id: 'delete-$kind',
          pubkey: 'author',
          createdAt: 30,
          kind: kind,
          tags: const [
            ['e', 'other-target'],
          ],
          content: '',
          sig: '',
        );
        expect(
          () => notifier.cacheThreadDeletions(
            [deletion],
            scopedTargetIds: {'queried-target'},
          ),
          throwsStateError,
        );
      }
    },
  );

  for (final known in [false, true]) {
    for (final survives in [false, true]) {
      test(
        'nested deletion reconciles broadcast summary (known: $known, survives: $survives)',
        () async {
          final broadcast = _event(
            id: 'broadcast',
            createdAt: 20,
            extraTags: const [
              ['e', 'root', '', 'root'],
              ['e', 'root', '', 'reply'],
              ['broadcast', '1'],
            ],
          );
          NostrEvent child(String id) => _event(
            id: id,
            createdAt: 30,
            extraTags: const [
              ['e', 'root', '', 'root'],
              ['e', 'broadcast', '', 'reply'],
            ],
          );
          final session = _RecordingRelaySessionNotifier(
            queryResults: [
              [
                broadcast,
                _event(id: 'root', createdAt: 10),
                _summary(
                  rootId: 'root',
                  replyCount: 1,
                  descendantCount: survives ? 3 : 2,
                ),
                _summary(
                  rootId: 'broadcast',
                  replyCount: survives ? 2 : 1,
                  descendantCount: 0,
                ),
                _bounds(),
              ],
              if (!known)
                [_summary(rootId: 'root', replyCount: survives ? 2 : 1)],
              [broadcast, if (survives) child('survivor')],
            ],
          );
          final container = _buildContainer(session);
          addTearDown(container.dispose);
          container.listen(channelMessagesProvider(_channelId), (_, _) {});
          await _pumpEventQueue();
          final notifier = container.read(
            channelMessagesProvider(_channelId).notifier,
          );
          if (known) {
            notifier.cacheConfirmedThreadReplies([child('deleted-child')]);
          }
          session.emit(
            NostrEvent(
              id: 'delete-child',
              pubkey: 'author',
              createdAt: 40,
              kind: EventKind.deletion,
              tags: const [
                ['h', _channelId],
                ['e', 'deleted-child'],
              ],
              content: '',
              sig: '',
            ),
          );
          await Future<void>.delayed(const Duration(milliseconds: 350));
          final entries = buildMainTimelineEntries(
            formatTimeline(
              container.read(channelMessagesProvider(_channelId)).value!,
            ),
            relaySummaries: notifier.threadSummaries,
          );
          expect(
            entries
                .singleWhere((e) => e.message.id == 'broadcast')
                .summary
                ?.replyCount,
            survives ? 1 : null,
          );
        },
      );
    }
  }

  test('deleted-target metadata disables only the owning empty root', () async {
    final session = _RecordingRelaySessionNotifier(
      queryResults: [
        [
          for (final id in ['owner', 'unrelated']) ...[
            _event(id: id, createdAt: 10),
            _summary(rootId: id, replyCount: 1),
          ],
          _bounds(),
        ],
        _DeletedTargetSummaryResponse(_summary(rootId: 'owner', replyCount: 0)),
      ],
    );
    final container = _buildContainer(session);
    addTearDown(container.dispose);
    container.listen(channelMessagesProvider(_channelId), (_, _) {});
    await _pumpEventQueue();
    session.emit(
      NostrEvent(
        id: 'delete',
        pubkey: 'author',
        createdAt: 100,
        kind: EventKind.deletion,
        tags: const [
          ['h', _channelId],
          ['e', 'unknown'],
        ],
        content: '',
        sig: '',
      ),
    );
    await Future<void>.delayed(const Duration(milliseconds: 300));
    final scans = session.queryFilters
        .where((filter) => filter.extensions.containsKey('depth_limit'))
        .toList();
    expect(scans, isEmpty);
    final summaries = container
        .read(channelMessagesProvider(_channelId).notifier)
        .threadSummaries;
    expect(summaries['owner']?.descendantCount, 0);
    expect(summaries['unrelated']?.descendantCount, 1);
    expect(summaries['unrelated']?.isCountPending, isFalse);
    expect(summaries['unrelated']?.isLowerBound, isFalse);
    final entries = buildMainTimelineEntries(
      formatTimeline(
        container.read(channelMessagesProvider(_channelId)).value!,
      ),
      relaySummaries: summaries,
    );
    expect(
      entries.singleWhere((entry) => entry.message.id == 'owner').summary,
      isNull,
    );
    expect(
      entries
          .singleWhere((entry) => entry.message.id == 'unrelated')
          .summary
          ?.replyCount,
      1,
    );
  });

  for (final newReplyExists in [false, true]) {
    test(
      'live summary after empty scan is reconciled (new reply: $newReplyExists)',
      () async {
        final recount = Completer<List<NostrEvent>>();
        final session = _RecordingRelaySessionNotifier(
          queryResults: [
            [_event(id: 'root', createdAt: 10), _bounds()],
            recount.future,
          ],
        );
        final container = _buildContainer(session);
        addTearDown(container.dispose);
        container.listen(channelMessagesProvider(_channelId), (_, _) {});
        await _pumpEventQueue();
        final notifier = container.read(
          channelMessagesProvider(_channelId).notifier,
        );
        notifier.cacheCompleteThreadQuery('root', {}, []);
        session.emit(_summary(rootId: 'root', replyCount: 1));
        expect(notifier.threadSummaries['root']!.descendantCount, 0);
        await Future<void>.delayed(const Duration(milliseconds: 300));
        expect(
          session.queryFilters.where(
            (f) => f.extensions.containsKey('depth_limit'),
          ),
          hasLength(1),
        );
        recount.complete([
          if (newReplyExists)
            _event(
              id: 'new-reply',
              createdAt: 30,
              extraTags: const [
                ['e', 'root', '', 'reply'],
              ],
            ),
        ]);
        await _pumpEventQueue();
        final entries = buildMainTimelineEntries(
          formatTimeline(
            container.read(channelMessagesProvider(_channelId)).value!,
          ),
          relaySummaries: notifier.threadSummaries,
        );
        expect(entries.single.summary?.replyCount, newReplyExists ? 1 : null);
      },
    );
  }

  for (final stillExists in [false, true]) {
    test(
      'live summary reconciles contradictory cached replies (exists: $stillExists)',
      () async {
        final reply = _event(
          id: 'cached-reply',
          createdAt: 20,
          extraTags: const [
            ['e', 'root', '', 'reply'],
          ],
        );
        final session = _RecordingRelaySessionNotifier(
          queryResults: [
            [_event(id: 'root', createdAt: 10), _bounds()],
            stillExists ? [reply] : <NostrEvent>[],
          ],
        );
        final container = _buildContainer(session);
        addTearDown(container.dispose);
        container.listen(channelMessagesProvider(_channelId), (_, _) {});
        await _pumpEventQueue();
        final notifier = container.read(
          channelMessagesProvider(_channelId).notifier,
        );
        notifier.cacheCompleteThreadQuery('root', {}, [reply]);
        session.emit(_summary(rootId: 'root', replyCount: 0));
        await Future<void>.delayed(const Duration(milliseconds: 300));
        final entries = buildMainTimelineEntries(
          formatTimeline(
            container.read(channelMessagesProvider(_channelId)).value!,
          ),
          relaySummaries: notifier.threadSummaries,
        );
        expect(
          entries.singleWhere((e) => e.message.id == 'root').summary == null,
          !stillExists,
        );
        expect(
          notifier.cachedThreadReplyIds('root').contains('cached-reply'),
          stillExists,
        );
        expect(notifier.threadSummaries['root']!.isCountPending, isFalse);
        expect(
          session.queryFilters.where(
            (f) => f.extensions.containsKey('depth_limit'),
          ),
          hasLength(1),
        );
      },
    );
  }

  for (final stillExists in [false, true]) {
    test(
      'WebSocket fallback recounts payload-only roots (exists: $stillExists)',
      () async {
        final reply = _event(
          id: 'live-reply',
          createdAt: 20,
          extraTags: const [
            ['e', 'root', '', 'reply'],
          ],
        );
        final session = _RecordingRelaySessionNotifier(
          queryResults: [
            [_event(id: 'root', createdAt: 10), _bounds()],
            Exception('NIP-CW unavailable'),
            stillExists ? [reply] : <NostrEvent>[],
          ],
          historyResults: [
            [_event(id: 'root', createdAt: 10)],
          ],
        );
        final container = _buildContainer(session);
        addTearDown(container.dispose);
        container.listen(channelMessagesProvider(_channelId), (_, _) {});
        await _pumpEventQueue();
        final notifier = container.read(
          channelMessagesProvider(_channelId).notifier,
        );
        session.emit(reply);
        expect(
          notifier.threadSummaries,
          isEmpty,
          reason: 'only reply payloads supply the initial badge',
        );
        session.setConnected(false);
        await _pumpEventQueue();
        session.setConnected(true);
        await Future<void>.delayed(const Duration(milliseconds: 300));
        final entries = buildMainTimelineEntries(
          formatTimeline(
            container.read(channelMessagesProvider(_channelId)).value!,
          ),
          relaySummaries: notifier.threadSummaries,
        );
        expect(
          entries.singleWhere((e) => e.message.id == 'root').summary == null,
          !stillExists,
        );
        expect(
          notifier.cachedThreadReplyIds('root').contains('live-reply'),
          stillExists,
        );
        expect(
          session.queryFilters.where(
            (f) => f.extensions.containsKey('depth_limit'),
          ),
          hasLength(1),
        );
      },
    );
  }

  for (final outcome in ['deleted', 'surviving', 'unavailable']) {
    test(
      'WebSocket fallback reconciles cached thread truth: $outcome',
      () async {
        final reply = _event(
          id: 'cached-reply',
          createdAt: 20,
          extraTags: const [
            ['e', 'root', '', 'reply'],
          ],
        );
        final recount = Completer<List<NostrEvent>>();
        final session = _RecordingRelaySessionNotifier(
          queryResults: [
            [_event(id: 'root', createdAt: 10), _bounds()],
            Exception('NIP-CW unavailable'),
            recount.future,
          ],
          historyResults: [
            [_event(id: 'root', createdAt: 10)],
          ],
        );
        final container = _buildContainer(session);
        addTearDown(container.dispose);
        container.listen(channelMessagesProvider(_channelId), (_, _) {});
        await _pumpEventQueue();
        final notifier = container.read(
          channelMessagesProvider(_channelId).notifier,
        );
        notifier.cacheCompleteThreadQuery('root', {}, [reply]);
        session.setConnected(false);
        await _pumpEventQueue();
        session.setConnected(true);
        await _pumpEventQueue();
        if (outcome != 'deleted') {
          expect(notifier.threadSummaries['root']!.isCountPending, isTrue);
        }
        await Future<void>.delayed(const Duration(milliseconds: 250));
        if (outcome == 'unavailable') {
          recount.completeError(Exception('thread query unavailable'));
        } else {
          recount.complete(outcome == 'deleted' ? [] : [reply]);
        }
        await _pumpEventQueue();
        final entries = buildMainTimelineEntries(
          formatTimeline(
            container.read(channelMessagesProvider(_channelId)).value!,
          ),
          relaySummaries: notifier.threadSummaries,
        );
        final summary = entries
            .singleWhere((entry) => entry.message.id == 'root')
            .summary;
        if (outcome == 'deleted') {
          expect(summary, isNull);
          expect(notifier.cachedThreadReplyIds('root'), isEmpty);
        } else {
          expect(summary, isNotNull);
          expect(summary!.replyCount, 1);
          expect(
            notifier.threadSummaries['root']!.isCountPending,
            outcome == 'unavailable',
          );
        }
        await Future<void>.delayed(const Duration(milliseconds: 250));
        expect(
          session.queryFilters.where(
            (filter) => filter.extensions.containsKey('depth_limit'),
          ),
          hasLength(1),
        );
      },
    );
  }

  test(
    'slow fallback history cannot supersede a newer complete thread query',
    () async {
      final reply = _event(
        id: 'cached-reply',
        createdAt: 20,
        extraTags: const [
          ['e', 'root', '', 'reply'],
        ],
      );
      final session = _RecordingRelaySessionNotifier(
        queryResults: [
          [_event(id: 'root', createdAt: 10), _bounds()],
          Exception('NIP-CW unavailable'),
        ],
      );
      final container = _buildContainer(session);
      addTearDown(container.dispose);
      container.listen(channelMessagesProvider(_channelId), (_, _) {});
      await _pumpEventQueue();
      final notifier = container.read(
        channelMessagesProvider(_channelId).notifier,
      );
      notifier.cacheCompleteThreadQuery('root', {}, [reply]);
      session.setConnected(false);
      await _pumpEventQueue();
      session.setConnected(true);
      await _pumpEventQueue();
      final snapshot = notifier.cachedThreadReplyIds('root');
      final version = notifier.beginThreadQuery('root');
      notifier.cacheCompleteThreadQuery(
        'root',
        snapshot,
        [],
        queryVersion: version,
      );
      session.completeHistory([_event(id: 'root', createdAt: 10)]);
      await Future<void>.delayed(const Duration(milliseconds: 300));
      expect(notifier.threadSummaries['root']!.descendantCount, 0);
      expect(notifier.threadSummaries['root']!.isCountPending, isFalse);
      expect(
        session.queryFilters.where(
          (f) => f.extensions.containsKey('depth_limit'),
        ),
        isEmpty,
      );
    },
  );

  test('reconnect pruning preserves duplicate deletion idempotence', () async {
    final survivor = _event(
      id: 'survivor',
      createdAt: 21,
      extraTags: const [
        ['e', 'root', '', 'reply'],
      ],
    );
    final session = _RecordingRelaySessionNotifier(
      queryResults: [
        [
          _event(id: 'root', createdAt: 10),
          _summary(rootId: 'root', replyCount: 2),
          _bounds(),
        ],
        [survivor],
        [
          _event(id: 'root', createdAt: 10),
          // A new reply arrived while disconnected; its payload is not cached.
          _summary(rootId: 'root', replyCount: 2),
          _bounds(),
        ],
        Exception('recount unavailable'),
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
        id: 'target',
        createdAt: 20,
        extraTags: const [
          ['e', 'root', '', 'reply'],
        ],
      ),
    ]);
    NostrEvent deletion(int kind) => NostrEvent(
      id: 'delete-$kind',
      pubkey: 'author',
      createdAt: 100,
      kind: kind,
      tags: const [
        ['h', _channelId],
        ['e', 'target'],
      ],
      content: '',
      sig: '',
    );
    final snapshot = notifier.cachedThreadReplyIds('root');
    session.emit(deletion(EventKind.deletion));
    final queryVersion = notifier.beginThreadQuery('root');
    notifier.cacheCompleteThreadQuery('root', snapshot, [
      survivor,
    ], queryVersion: queryVersion);
    await Future<void>.delayed(const Duration(milliseconds: 300));
    expect(notifier.cachedThreadReplyIds('root'), isNot(contains('target')));
    session.setConnected(false);
    await _pumpEventQueue();
    session.setConnected(true);
    await _pumpEventQueue();
    session.emit(_event(id: 'after-reconnect', createdAt: 101));
    expect(
      container
          .read(channelMessagesProvider(_channelId))
          .value!
          .any((event) => event.id == 'delete-5'),
      isFalse,
      reason: 'the marker payload was pruned on reconnect',
    );
    expect(notifier.threadSummaries['root']!.descendantCount, 2);
    session.emit(deletion(EventKind.nip29DeleteEvent));
    await Future<void>.delayed(const Duration(milliseconds: 300));
    expect(notifier.threadSummaries['root']!.descendantCount, 2);
    expect(notifier.threadSummaries['root']!.isLowerBound, isFalse);
    expect(
      session.queryFilters.where(
        (filter) => filter.extensions.containsKey('depth_limit'),
      ),
      hasLength(1),
    );
  });

  for (final fetched in [false, true]) {
    test(
      'duplicate deletion markers decrement a partial summary only once (fetched: $fetched)',
      () async {
        final session = _RecordingRelaySessionNotifier(
          queryResults: [
            [
              _event(id: 'root', createdAt: 10),
              _summary(rootId: 'root', replyCount: 2),
              _bounds(),
            ],
            Exception('recount unavailable'),
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
            id: 'target',
            createdAt: 20,
            extraTags: const [
              ['e', 'root', '', 'reply'],
            ],
          ),
        ]);
        for (final kind in [
          EventKind.deletion,
          EventKind.deletion,
          EventKind.nip29DeleteEvent,
        ]) {
          final deletion = NostrEvent(
            id: 'delete-$kind',
            pubkey: 'author',
            createdAt: 100,
            kind: kind,
            tags: const [
              ['h', _channelId],
              ['e', 'target'],
            ],
            content: '',
            sig: '',
          );
          if (fetched) {
            notifier.cacheThreadDeletions([deletion]);
          } else {
            session.emit(deletion);
          }
        }
        await Future<void>.delayed(const Duration(milliseconds: 300));
        expect(notifier.threadSummaries['root']?.descendantCount, 1);
        expect(
          session.queryFilters.where(
            (filter) => filter.extensions.containsKey('depth_limit'),
          ),
          hasLength(1),
        );
      },
    );
  }

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
  int? descendantCount,
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
      'descendant_count': descendantCount ?? replyCount,
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

class _DeletedTargetSummaryResponse {
  final NostrEvent summary;
  _DeletedTargetSummaryResponse(this.summary);
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
    if (result is _DeletedTargetSummaryResponse) {
      // Normal history cannot return soft-deleted reply payloads.
      return filters.single.extensions['resolve_thread_roots'] == true
          ? [result.summary]
          : [];
    }
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
