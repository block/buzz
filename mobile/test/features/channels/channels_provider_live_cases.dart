part of 'channels_provider_test.dart';

void _liveSubscriptionTests() {
  const myPk = 'me';

  test(
    'subscribes once for joined non-archived channels without blocking snapshot',
    () async {
      final session = _FakeRelaySession(
        memberships: [
          _membership(_channelA, myPk),
          _membership(_channelB, myPk),
          _membership(_channelD, myPk),
        ],
        metadata: [
          _meta(id: _channelA, name: 'general'),
          _meta(id: _channelB, name: 'random'),
          // channelD metadata missing -> won't appear in channel list
        ],
      );
      session.pauseNextSubscribe();
      final container = _buildContainer(session: session);
      addTearDown(container.dispose);

      final channels = await container.read(channelsProvider.future);

      expect(channels.map((channel) => channel.id).toSet(), {
        _channelA,
        _channelB,
      });
      await session.nextSubscribeStarted;
      expect(session.subscribeFilters, hasLength(1));
      expect(session.subscribeFilters.single.tags['#h'], [
        _channelA,
        _channelB,
      ]);
      expect(
        session.subscribeFilters.single.kinds,
        EventKind.channelEventKinds,
      );
      expect(session.subscribeFilters.single.limit, 0);

      session.resumePausedSubscribe();
      await _waitUntil(() => session.activeChannels.length == 2);
    },
  );

  test('chunks live subscriptions at the relay explicit-channel cap', () async {
    final channelIds = [for (var i = 0; i < 257; i++) _generatedChannelId(i)];
    final session = _FakeRelaySession(
      memberships: [for (final id in channelIds) _membership(id, myPk)],
      metadata: [for (final id in channelIds) _meta(id: id, name: id)],
    );
    final container = _buildContainer(session: session);
    addTearDown(container.dispose);

    final channels = await container.read(channelsProvider.future);
    expect(channels, hasLength(257));
    await _waitUntil(() => session.subscribeFilters.length == 3);

    expect(
      session.subscribeFilters.map((filter) => filter.tags['#h']!.length),
      [128, 128, 1],
    );
    expect(
      session.subscribeFilters.expand((filter) => filter.tags['#h']!).toSet(),
      channelIds.toSet(),
    );
  });

  test(
    'refreshing an unchanged channel set issues zero new live REQs',
    () async {
      final session = _FakeRelaySession(
        memberships: [
          _membership(_channelA, myPk),
          _membership(_channelB, myPk),
        ],
        metadata: [
          _meta(id: _channelA, name: 'general'),
          _meta(id: _channelB, name: 'random'),
        ],
      );
      final container = _buildContainer(session: session);
      addTearDown(container.dispose);

      await container.read(channelsProvider.future);
      await _waitUntil(() => session.activeSubscriptionCount == 1);
      final initialSubscribeCount = session.totalSubscribeCount;

      await container.read(channelsProvider.notifier).refresh();
      await _settle();

      expect(session.totalSubscribeCount, initialSubscribeCount);
      expect(session.unsubscribeCount, 0);
      expect(session.subscribeFilters, hasLength(1));
    },
  );

  test('changing a live chunk replaces only that chunk', () async {
    final channelIds = [for (var i = 0; i < 129; i++) _generatedChannelId(i)];
    final addedId = _generatedChannelId(999);
    final session = _FakeRelaySession(
      memberships: [for (final id in channelIds) _membership(id, myPk)],
      metadata: [for (final id in channelIds) _meta(id: id, name: id)],
    );
    final container = _buildContainer(session: session);
    addTearDown(container.dispose);

    await container.read(channelsProvider.future);
    await _waitUntil(() => session.activeSubscriptionCount == 2);

    session.memberships = [
      for (final id in [...channelIds.take(128), addedId])
        _membership(id, myPk),
    ];
    session.metadata = [
      for (final id in [...channelIds.take(128), addedId])
        _meta(id: id, name: id),
    ];
    await container.read(channelsProvider.notifier).refresh();
    await _waitUntil(() => session.totalSubscribeCount == 3);

    expect(session.activeSubscriptionCount, 2);
    expect(session.unsubscribeCount, 1);
    expect(session.activeChannels, {...channelIds.take(128), addedId});
  });

  test(
    'front-sorting membership insertion retains coverage while replacement waits',
    () async {
      final channelIds = [
        for (var i = 1; i <= 129; i++) _generatedChannelId(i),
      ];
      final addedId = _generatedChannelId(0);
      final session = _FakeRelaySession(
        memberships: [for (final id in channelIds) _membership(id, myPk)],
        metadata: [for (final id in channelIds) _meta(id: id, name: id)],
      );
      final container = _buildContainer(session: session);
      addTearDown(container.dispose);
      await container.read(channelsProvider.future);
      await _waitUntil(() => session.activeSubscriptionCount == 2);

      session.memberships.add(_membership(addedId, myPk));
      session.metadata.add(_meta(id: addedId, name: addedId));
      session.pauseNextSubscribe();
      await container.read(channelsProvider.notifier).refresh();
      await session.nextSubscribeStarted;
      try {
        expect(session.activeChannels, channelIds.toSet());
        expect(session.unsubscribeCount, 0);
        session.emit(_liveMessage(channelIds.last));
        expect(
          container
              .read(channelsProvider)
              .requireValue
              .firstWhere((channel) => channel.id == channelIds.last)
              .lastMessageAt
              ?.millisecondsSinceEpoch,
          20000,
        );
      } finally {
        session.resumePausedSubscribe();
      }
      await _waitUntil(() => session.unsubscribeCount == 2);
      expect(session.totalSubscribeCount, 4);
      expect(session.activeSubscriptionCount, 2);
      expect(session.activeChannels, {...channelIds, addedId});
    },
  );

  test(
    'failed replacement keeps desired coverage and ignores departed channels',
    () async {
      final session = _FakeRelaySession(
        memberships: [
          _membership(_channelA, myPk),
          _membership(_channelB, myPk),
        ],
        metadata: [
          _meta(id: _channelA, name: 'a'),
          _meta(id: _channelB, name: 'b'),
        ],
      );
      final container = _buildContainer(session: session);
      addTearDown(container.dispose);
      await container.read(channelsProvider.future);
      await _waitUntil(() => session.activeSubscriptionCount == 1);
      session.memberships = [
        _membership(_channelB, myPk),
        _membership(_channelD, myPk),
      ];
      session.metadata = [
        _meta(id: _channelB, name: 'b'),
        _meta(id: _channelD, name: 'd'),
      ];
      session.subscribeFailures = 1;
      await container.read(channelsProvider.notifier).refresh();
      await _settle();
      expect(session.activeChannels, {_channelA, _channelB});
      expect(session.unsubscribeCount, 0);
      final requests = session.membershipRequestCount;
      session.emit(_liveMessage(_channelA));
      await _settle();
      expect(session.membershipRequestCount, requests);
      session.emit(_liveMessage(_channelB));
      expect(
        container
            .read(channelsProvider)
            .requireValue
            .firstWhere((channel) => channel.id == _channelB)
            .lastMessageAt
            ?.millisecondsSinceEpoch,
        20000,
      );

      await container.read(channelsProvider.notifier).refresh();
      await _waitUntil(() => session.unsubscribeCount == 1);
      expect(session.activeChannels, {_channelB, _channelD});
      expect(session.activeSubscriptionCount, 1);
    },
  );

  test(
    'terminal closure preserves fallback until reconnect retries coverage',
    () async {
      final channelIds = [
        for (var i = 1; i <= 129; i++) _generatedChannelId(i),
      ];
      final addedId = _generatedChannelId(0);
      final session = _FakeRelaySession(
        memberships: [for (final id in channelIds) _membership(id, myPk)],
        metadata: [for (final id in channelIds) _meta(id: id, name: id)],
      );
      final container = _buildContainer(session: session);
      addTearDown(container.dispose);
      await container.read(channelsProvider.future);
      await _waitUntil(() => session.activeSubscriptionCount == 2);

      session.memberships.add(_membership(addedId, myPk));
      session.metadata.add(_meta(id: addedId, name: addedId));
      session.subscribeFailures = 1;
      await container.read(channelsProvider.notifier).refresh();
      await _settle();
      expect(session.activeChannels, containsAll(channelIds));

      session.closeSubscriptionContaining(
        channelIds.last,
        'restricted: terminal closure',
      );
      await _settle();
      // The rejected chunk itself is gone, but the previously retained
      // fallback must still cover its original first 128 channels.
      expect(session.activeChannels, channelIds.take(128).toSet());
      expect(session.activeChannels, isNot(contains(addedId)));
      session.setStatus(SessionStatus.disconnected);
      session.setStatus(SessionStatus.connected);
      await _waitUntil(() => session.activeChannels.contains(addedId));

      expect(session.activeChannels, containsAll({...channelIds, addedId}));
      expect(session.activeSubscriptionCount, 2);
    },
  );

  test(
    'partial replacement churn keeps complete coverage with bounded fallbacks',
    () async {
      var channelIds = [for (var i = 100; i < 356; i++) _generatedChannelId(i)];
      final session = _FakeRelaySession(
        memberships: [for (final id in channelIds) _membership(id, myPk)],
        metadata: [for (final id in channelIds) _meta(id: id, name: id)],
      );
      final container = _buildContainer(session: session);
      addTearDown(container.dispose);
      await container.read(channelsProvider.future);
      await _waitUntil(() => session.activeSubscriptionCount == 2);

      for (var cycle = 0; cycle < 7; cycle++) {
        final addedId = _generatedChannelId(99 - cycle);
        channelIds = [addedId, ...channelIds.take(255)];
        session.memberships = [
          for (final id in channelIds) _membership(id, myPk),
        ];
        session.metadata = [
          for (final id in channelIds) _meta(id: id, name: id),
        ];
        session.successfulSubscribesBeforeFailure = 1;
        session.subscribeFailures = 1;
        await container.read(channelsProvider.notifier).refresh();
        await _settle();
        expect(session.activeChannels, containsAll(channelIds));
        expect(session.activeSubscriptionCount, lessThanOrEqualTo(3));
      }
    },
  );

  test(
    'retired replacement cleans up even when the next generation disconnects',
    () async {
      final session = _FakeRelaySession(
        memberships: [_membership(_channelA, myPk)],
        metadata: [_meta(id: _channelA, name: 'a')],
      );
      final container = _buildContainer(session: session);
      addTearDown(container.dispose);
      await container.read(channelsProvider.future);
      await _waitUntil(() => session.activeSubscriptionCount == 1);
      session.memberships.add(_membership(_channelB, myPk));
      session.metadata.add(_meta(id: _channelB, name: 'b'));
      session.pauseNextSubscribe();
      await container.read(channelsProvider.notifier).refresh();
      await session.nextSubscribeStarted;
      try {
        expect(session.activeChannels, {_channelA});
        session.memberships = [];
        session.metadata = [];
        await container.read(channelsProvider.notifier).refresh();
        session.setStatus(SessionStatus.disconnected);
      } finally {
        session.resumePausedSubscribe();
      }
      await _waitUntil(() => session.unsubscribeCount == 2);
      expect(session.activeChannels, isEmpty);
      expect(session.activeSubscriptionCount, 0);
    },
  );

  test('disposal retires retained and in-flight replacement chunks', () async {
    final session = _FakeRelaySession(
      memberships: [_membership(_channelA, myPk)],
      metadata: [_meta(id: _channelA, name: 'a')],
    );
    final container = _buildContainer(session: session);
    await container.read(channelsProvider.future);
    await _waitUntil(() => session.activeSubscriptionCount == 1);
    session.memberships.add(_membership(_channelB, myPk));
    session.metadata.add(_meta(id: _channelB, name: 'b'));
    session.pauseNextSubscribe();
    await container.read(channelsProvider.notifier).refresh();
    await session.nextSubscribeStarted;
    container.dispose();
    session.resumePausedSubscribe();
    await _waitUntil(() => session.unsubscribeCount == 2);
    expect(session.activeSubscriptionCount, 0);
  });

  test('empty channel refresh removes every retained live chunk', () async {
    final session = _FakeRelaySession(
      memberships: [_membership(_channelA, myPk), _membership(_channelB, myPk)],
      metadata: [
        _meta(id: _channelA, name: 'general'),
        _meta(id: _channelB, name: 'random'),
      ],
    );
    final container = _buildContainer(session: session);
    addTearDown(container.dispose);

    await container.read(channelsProvider.future);
    await _waitUntil(() => session.activeSubscriptionCount == 1);
    session.memberships = [];
    session.metadata = [];

    await container.read(channelsProvider.notifier).refresh();
    await _waitUntil(() => session.activeSubscriptionCount == 0);

    expect(session.activeChannels, isEmpty);
    expect(session.unsubscribeCount, 1);
  });

  test(
    'community switch retires a detached old-scope live subscription',
    () async {
      final session = _FakeRelaySession(
        memberships: [_membership(_channelA, myPk)],
        metadata: [_meta(id: _channelA, name: 'general')],
      );
      session.pauseNextSubscribe();
      final container = _buildContainer(session: session);
      addTearDown(container.dispose);

      expect(
        (await container.read(channelsProvider.future)).single.id,
        _channelA,
      );
      await session.nextSubscribeStarted;

      session.memberships = [_membership(_channelB, myPk)];
      session.metadata = [_meta(id: _channelB, name: 'random')];
      container
          .read(relayConfigProvider.notifier)
          .update(baseUrl: 'https://new-community.example');
      await container.read(channelsProvider.future);

      session.resumePausedSubscribe();
      await _waitUntil(() => session.activeChannels.contains(_channelB));

      expect(session.activeChannels, {_channelB});
      expect(session.activeSubscriptionCount, 1);
      expect(session.unsubscribeCount, 1);
    },
  );
}

NostrEvent _liveMessage(String channelId) => NostrEvent(
  id: 'live-$channelId',
  pubkey: 'alice',
  createdAt: 20,
  kind: EventKind.streamMessageV2,
  tags: [
    ['h', channelId],
  ],
  content: 'live update',
  sig: 'sig',
);
