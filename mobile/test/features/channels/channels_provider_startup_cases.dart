part of 'channels_provider_test.dart';

void _channelStartupCases() {
  test(
    'starts independent channel reads together and waits for all of them',
    () async {
      final session = _FakeRelaySession(
        memberships: [_membership(_channelA, 'me')],
        metadata: [_meta(id: _channelA, name: 'general')],
      );
      session.pauseNextHiddenDmQuery();
      session.pauseNextMemberCountQuery();
      session.pauseNextHuddleStartQuery();
      final allStarted = Future.wait([
        session.nextHiddenDmQueryStarted,
        session.nextMemberCountQueryStarted,
        session.nextHuddleStartQueryStarted,
      ]);
      final container = _buildContainer(session: session);
      addTearDown(container.dispose);
      final loaded = container.read(channelsProvider.future);

      try {
        // None of the responses can complete until all requests have started.
        // Serial reads cannot pass this assertion, regardless of machine speed.
        await allStarted.timeout(const Duration(seconds: 2));
        expect(container.read(channelsProvider).isLoading, isTrue);
      } finally {
        session.resumePausedMemberCountQuery();
        session.resumePausedHuddleStartQuery();
        session.resumePausedHiddenDmQuery();
      }

      final channels = await loaded;
      expect(channels.map((channel) => channel.id), [_channelA]);
      expect(channels.single.memberCount, 1);
    },
  );
}
