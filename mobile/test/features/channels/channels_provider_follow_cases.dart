part of 'channels_provider_test.dart';

/// Merged thread follow choices a test sets directly, as a peer's would
/// arrive through read state.
class _TestFollows extends ThreadFollowsNotifier {
  @override
  ThreadFollowsState build() => const ThreadFollowsState(incomplete: false);

  void choose({
    Set<String> followed = const {},
    Set<String> unfollowed = const {},
  }) => state = ThreadFollowsState(
    followedRootIds: followed,
    unfollowedRootIds: unfollowed,
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

    Future<(ProviderContainer, _FakeRelaySession, _TestFollows)> start() async {
      final session = _FakeRelaySession(
        memberships: [_membership(_channelA, 'me')],
        metadata: [_meta(id: _channelA, name: 'general')],
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
  });
}
