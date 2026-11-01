part of 'channels_provider_test.dart';

void _membershipDeltaCases() {
  const myPk = 'me';
  const channelC = '33333333-3333-4333-8333-333333333333';
  const channelE = '55555555-5555-4555-8555-555555555555';

  List<String> listedIds(ProviderContainer container) => container
      .read(channelsProvider)
      .requireValue
      .map((channel) => channel.id)
      .toList();

  test('a refresh reads only the memberships that changed', () async {
    final session = _FakeRelaySession(
      memberships: [
        for (final id in [_channelA, _channelB, channelC, _channelD])
          _membership(id, myPk, createdAt: 100),
      ],
      metadata: [
        for (final (id, name) in [
          (_channelA, 'a'),
          (_channelB, 'b'),
          (channelC, 'c'),
          (_channelD, 'd'),
          (channelE, 'e'),
        ])
          _meta(id: id, name: name, visibility: 'private'),
      ],
    );
    var removed = <Map<String, Object>>[];
    var asOf = 10000;
    session.membershipRemovals = (_) => {'asOf': asOf, 'removed': removed};
    final container = _buildContainer(session: session);
    addTearDown(container.dispose);

    await container.read(channelsProvider.future);
    expect(session.membershipQueryFilters.map((f) => f.since).toSet(), {null});
    expect(
      listedIds(container),
      unorderedEquals([_channelA, _channelB, channelC, _channelD]),
    );

    // B: removed. C: deleted. D: removed, then added back. E: joined.
    session.memberships = [
      _membership(_channelA, myPk, createdAt: 100),
      _membership(_channelD, myPk, createdAt: 9700),
      _membership(channelE, myPk, createdAt: 9800),
    ];
    removed = [
      {'channelId': _channelB, 'at': 9500, 'reason': 'removed'},
      {'channelId': channelC, 'at': 9600, 'reason': 'deleted'},
      {'channelId': _channelD, 'at': 9650, 'reason': 'removed'},
    ];
    asOf = 10500;
    await container.read(channelsProvider.notifier).refresh();

    // The roster read starts 960s before the relay clock of the last read,
    // so A's unchanged roster is not downloaded again.
    expect(session.membershipQueryFilters.last.since, 10000 - 960);
    expect(session.membershipRemovalSinces.last, 10000 - 960);
    expect(
      listedIds(container),
      unorderedEquals([_channelA, _channelD, channelE]),
    );

    // The next read starts from the new relay clock.
    await container.read(channelsProvider.notifier).refresh();
    expect(session.membershipQueryFilters.last.since, 10500 - 960);
    expect(
      listedIds(container),
      unorderedEquals([_channelA, _channelD, channelE]),
    );
  });

  test('a refused removed read falls back to the full read', () async {
    final legacy = _FakeRelaySession(
      memberships: [_membership(_channelA, myPk, createdAt: 100)],
      metadata: [_meta(id: _channelA, name: 'a')],
    );
    final legacyContainer = _buildContainer(session: legacy);
    addTearDown(legacyContainer.dispose);

    // An older relay answers 403, and is not asked again for a while.
    await legacyContainer.read(channelsProvider.future);
    await legacyContainer.read(channelsProvider.notifier).refresh();
    expect(legacy.membershipRemovalSinces, hasLength(1));
    expect(legacy.membershipQueryFilters.map((f) => f.since).toSet(), {null});
    expect(listedIds(legacyContainer), [_channelA]);

    // Too many changes (413) costs only that read its shortcut: the anchor
    // for the full read, which asks from now, still succeeds.
    final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
    var tooMany = false;
    final session = _FakeRelaySession(
      memberships: [_membership(_channelA, myPk, createdAt: 100)],
      metadata: [
        _meta(id: _channelA, name: 'a'),
        _meta(id: _channelB, name: 'b'),
      ],
    );
    session.membershipRemovals = (since) {
      if (tooMany && since < now - 60) {
        throw RelayException(413, '{"error":"too many membership changes"}');
      }
      return {'asOf': now, 'removed': const <Object>[]};
    };
    final container = _buildContainer(session: session);
    addTearDown(container.dispose);
    await container.read(channelsProvider.future);

    tooMany = true;
    session.memberships = [_membership(_channelB, myPk, createdAt: 200)];
    var before = session.membershipQueryFilters.length;
    await container.read(channelsProvider.notifier).refresh();
    expect(
      session.membershipQueryFilters.skip(before).map((f) => f.since).toSet(),
      {now - 960, null},
    );
    expect(listedIds(container), [_channelB]);

    tooMany = false;
    before = session.membershipQueryFilters.length;
    await container.read(channelsProvider.notifier).refresh();
    expect(
      session.membershipQueryFilters.skip(before).map((f) => f.since).toSet(),
      {now - 960},
    );
    expect(listedIds(container), [_channelB]);
  });
}
