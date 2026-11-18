part of 'channels_provider_test.dart';

/// Merged thread follow choices a test sets directly, as a peer's would
/// arrive through read state.
class _TestFollows extends ThreadFollowsNotifier {
  @override
  ThreadFollowsState build() => const ThreadFollowsState(incomplete: false);

  void choose({
    Set<String> followed = const {},
    Set<String> unfollowed = const {},
    Map<String, String> channelByRoot = const {},
  }) => state = ThreadFollowsState(
    followedRootIds: followed,
    unfollowedRootIds: unfollowed,
    channelByRoot: channelByRoot,
    incomplete: false,
  );
}

void _threadFollowCases() {
  group('thread follow re-evaluation', () {
    final root = 'f' * 64;
    final base = DateTime.now().millisecondsSinceEpoch ~/ 1000 - 100;
    final reply = NostrEvent(
      id: 'follow-reply',
      pubkey: 'alice',
      createdAt: base + 50,
      kind: 9,
      tags: [
        ['h', _channelA],
        ['e', root, '', 'root'],
        ['e', root, '', 'reply'],
      ],
      content: 'reply',
      sig: 'sig',
    );

    Future<(ProviderContainer, _FakeRelaySession, _TestFollows)> start({
      String channelType = 'stream',
    }) async {
      final session = _FakeRelaySession(
        memberships: [_membership(_channelA, 'me')],
        metadata: [
          _meta(id: _channelA, name: 'general', channelType: channelType),
        ],
      );
      final follows = _TestFollows();
      final container = _buildContainer(session: session, follows: follows);
      addTearDown(container.dispose);
      await container.read(channelsProvider.future);
      await _settle();
      return (container, session, follows);
    }

    Map<String, ObservedUnreadEvent> observed(ProviderContainer container) =>
        container
            .read(channelsProvider.notifier)
            .observedUnreadEventsByChannel[_channelA] ??
        const {};

    test('an already-counted reply stops counting once its thread is '
        'unfollowed', () async {
      final (container, session, follows) = await start();
      follows.choose(followed: {root});
      session.emit(reply);
      await _settle();
      expect(observed(container), contains(reply.id));

      follows.choose(unfollowed: {root});
      await _settle();
      expect(observed(container), isNot(contains(reply.id)));
    });

    test('a reply skipped while unfollowed counts once the thread is '
        'followed', () async {
      final (container, session, follows) = await start();
      follows.choose(unfollowed: {root});
      session.emit(reply);
      await _settle();
      expect(observed(container), isNot(contains(reply.id)));

      session.recentMessages = [reply];
      follows.choose(followed: {root});
      await _settle();
      await container.read(channelsProvider.notifier).waitForUnreadCatchUp();
      expect(observed(container), contains(reply.id));
    });

    /// Applies [before], then [widen], after a first catch-up and returns the `since` of the
    /// next catch-up's filters, per channel.
    Future<Map<String, Set<int?>>> sinceAfter(
      void Function(_TestFollows follows) widen, {
      void Function(_TestFollows follows)? before,
    }) async {
      final session = _FakeRelaySession(
        memberships: [
          _membership(_channelA, 'me'),
          _membership(_channelB, 'me'),
        ],
        metadata: [
          _meta(id: _channelA, name: 'general'),
          _meta(id: _channelB, name: 'random'),
        ],
      );
      final follows = _TestFollows();
      final container = _buildContainer(session: session, follows: follows);
      addTearDown(container.dispose);
      bool isUnread(List<NostrFilter> batch) =>
          batch.isNotEmpty && batch.every((filter) => filter.since != null);
      await container.read(channelsProvider.future);
      await _waitUntil(() => session.queryBatches.any(isUnread));
      await _settle();
      before?.call(follows);
      await _settle();

      final count = session.queryBatches.where(isUnread).length;
      widen(follows);
      await _waitUntil(
        () => session.queryBatches.where(isUnread).length > count,
      );
      await _settle();
      return {
        for (final channelId in [_channelA, _channelB])
          channelId: {
            for (final filter in session.queryBatches.lastWhere(isUnread))
              if (filter.tags['#h']?.contains(channelId) ?? false) filter.since,
          },
      };
    }

    test(
      'a newly followed thread re-catches up only its own channel',
      () async {
        final since = await sinceAfter(
          (follows) => follows.choose(
            followed: {root},
            channelByRoot: {root: _channelA},
          ),
        );
        // Never read: the followed thread's channel asks for its whole window
        // again; the other channel keeps its incremental cursor.
        expect(since[_channelA], {0});
        expect(
          since[_channelB],
          allOf(isNotEmpty, everyElement(greaterThan(0))),
        );
      },
    );

    test('a followed thread in an unknown channel re-catches up every '
        'channel', () async {
      final since = await sinceAfter(
        (follows) => follows.choose(followed: {root}),
      );
      expect(since[_channelA], {0});
      expect(since[_channelB], {0});
    });

    test('a removed unfollow re-catches up the channel it had', () async {
      final since = await sinceAfter(
        before: (follows) => follows.choose(
          unfollowed: {root},
          channelByRoot: {root: _channelA},
        ),
        // Back to automatic: the next state no longer names the channel.
        (follows) => follows.choose(),
      );
      expect(since[_channelA], {0});
      expect(since[_channelB], allOf(isNotEmpty, everyElement(greaterThan(0))));
    });

    test('an unfollow never mutes an ordinary DM reply', () async {
      final (container, session, follows) = await start(channelType: 'dm');
      // The reader took part in the thread, then unfollowed it.
      session.emit(
        NostrEvent(
          id: 'own-reply',
          pubkey: 'me',
          createdAt: base + 40,
          kind: 9,
          tags: reply.tags,
          content: 'mine',
          sig: 'sig',
        ),
      );
      follows.choose(unfollowed: {root});
      await _settle();
      session.emit(reply);
      await _settle();
      expect(observed(container), contains(reply.id));
    });
  });
}
