part of 'channels_provider_test.dart';

void _huddleNavigationCases() {
  test(
    'a newer receipt snapshot is reconciled after the pending pass',
    () async {
      const roomC = '33333333-3333-4333-8333-333333333333';
      final creator = 'a' * 64;
      NostrEvent receipt(String id) => NostrEvent(
        id: 'receipt-$id',
        pubkey: creator,
        createdAt: 1,
        kind: 9007,
        sig: '',
        content: '',
        tags: [
          ['h', id],
          ['visibility', 'private'],
          ['channel_type', 'stream'],
          ['about', 'Buzz Huddle (buzz.huddles/v1)\nparent:$_channelA'],
        ],
      );
      NostrEvent meta(String id) => _meta(
        id: id,
        name: 'Renamed room',
        visibility: 'private',
        creator: creator,
      );
      final session = _FakeRelaySession(
        memberships: [_membership(_channelB, 'me')],
        metadata: [meta(_channelB)],
        creations: [receipt(_channelB), receipt(roomC)],
      );
      final gate = session.creationGate = Completer<void>();
      final container = _buildContainer(session: session);
      addTearDown(container.dispose);
      await container.read(channelsProvider.future);
      await _settle();
      expect(session.creationReads, 1);
      session.memberships = [
        _membership(_channelB, 'me'),
        _membership(roomC, 'me'),
      ];
      session.metadata = [meta(_channelB), meta(roomC)];
      await container.read(channelsProvider.notifier).refresh();
      await _settle();
      expect(container.read(channelsProvider).requireValue, hasLength(2));
      gate.complete();
      await _settle();
      expect(container.read(channelsProvider).requireValue, isEmpty);
      expect(session.creationReads, 2);
    },
  );

  test('receipt batches stop after switching communities', () async {
    final ids = List.generate(129, (i) => 'private-$i');
    final session = _FakeRelaySession(
      memberships: [for (final id in ids) _membership(id, 'me')],
      metadata: [
        for (final id in ids) _meta(id: id, name: id, visibility: 'private'),
      ],
    );
    final gate = session.creationGate = Completer<void>();
    final container = _buildContainer(session: session);
    addTearDown(container.dispose);
    expect(await container.read(channelsProvider.future), hasLength(129));
    await _settle();
    expect(session.creationReads, 1);
    session.memberships = [_membership(_channelA, 'me')];
    session.metadata = [_meta(id: _channelA, name: 'new community')];
    container
        .read(relayConfigProvider.notifier)
        .update(baseUrl: 'https://other.example');
    await container.read(channelsProvider.future);
    gate.complete();
    await _settle();
    expect(session.creationReads, 1);
    expect(container.read(channelsProvider).requireValue.single.id, _channelA);
    expect(container.read(huddleHistoryMembershipProvider(_channelB)), isFalse);
  });

  test(
    'old named rooms require creator-matched Huddle creation evidence',
    () async {
      final creator = 'a' * 64;
      final marker = 'Buzz Huddle (buzz.huddles/v1)\nparent:$_channelA';
      for (final variant in [
        'valid',
        'renamed',
        'wrong creator',
        'missing creator',
        'ordinary',
        'invalid parent',
        'duplicate coordinate',
        'open creation',
        'forum creation',
        'description only',
      ]) {
        final receipt = NostrEvent(
          id: 'creation',
          pubkey: variant == 'wrong creator' ? 'b' * 64 : creator,
          createdAt: 1,
          kind: 9007,
          sig: 'sig',
          content: '',
          tags: [
            ['h', _channelB],
            if (variant == 'duplicate coordinate') ['h', _channelA],
            ['visibility', variant == 'open creation' ? 'open' : 'private'],
            ['channel_type', variant == 'forum creation' ? 'forum' : 'stream'],
            [
              'about',
              variant == 'ordinary'
                  ? ''
                  : variant == 'invalid parent'
                  ? 'Buzz Huddle (buzz.huddles/v1)\nparent:invalid'
                  : marker,
            ],
          ],
        );
        final session = _FakeRelaySession(
          memberships: [
            _membership(_channelA, 'me'),
            _membership(_channelB, 'me'),
          ],
          metadata: [
            _meta(id: _channelA, name: 'general'),
            _meta(
              id: _channelB,
              name: variant == 'renamed' ? 'Renamed room' : 'Huddle',
              visibility: 'private',
              ttlSeconds: 3600,
              creator: variant == 'missing creator' ? null : creator,
              description: marker,
            ),
          ],
          creations: variant == 'description only' ? [] : [receipt],
        );
        session.creationGate = Completer<void>();
        final container = _buildContainer(session: session);
        try {
          final initial = await container.read(channelsProvider.future);
          expect(initial.any((c) => c.id == _channelA), isTrue);
          expect(initial.any((c) => c.id == _channelB), isTrue);
          session.creationGate!.complete();
          await Future<void>(() {});
          await container.pump();
          final channels = container.read(channelsProvider).requireValue;
          final hidden = variant == 'valid' || variant == 'renamed';
          expect(
            channels.any((c) => c.id == _channelB),
            !hidden,
            reason: variant,
          );
          expect(
            container.read(channelViewerIsMemberProvider(_channelB)),
            isTrue,
          );
          session.failCreations = true;
          await container.read(channelsProvider.notifier).refresh();
          expect(session.creationReads, 1);
          expect(
            container
                .read(channelsProvider)
                .requireValue
                .any((c) => c.id == _channelB),
            !hidden,
            reason: '$variant after refresh',
          );
        } finally {
          container.dispose();
        }
      }
    },
  );

  test(
    'old generated Huddle rooms stay hidden without losing membership',
    () async {
      final cases = [
        (
          name: 'huddle-22222222',
          type: 'stream',
          visibility: 'private',
          ttl: 3600,
          hidden: true,
        ),
        (
          name: 'huddle-22222222',
          type: 'stream',
          visibility: 'private',
          ttl: 60,
          hidden: true,
        ),
        (
          name: 'huddle-22222222',
          type: 'stream',
          visibility: 'private',
          ttl: null,
          hidden: false,
        ),
        (
          name: 'huddle-22222222',
          type: 'stream',
          visibility: 'private',
          ttl: 0,
          hidden: false,
        ),
        (
          name: 'huddle-22222222',
          type: 'stream',
          visibility: 'private',
          ttl: -1,
          hidden: false,
        ),
        (
          name: 'huddle-22222222',
          type: 'stream',
          visibility: 'private',
          ttl: 2147483648,
          hidden: false,
        ),
        (
          name: 'huddle-22222222',
          type: 'stream',
          visibility: 'open',
          ttl: 3600,
          hidden: false,
        ),
        (
          name: 'huddle-22222222',
          type: 'forum',
          visibility: 'private',
          ttl: 3600,
          hidden: false,
        ),
        (
          name: 'huddle-other-id',
          type: 'stream',
          visibility: 'private',
          ttl: 3600,
          hidden: false,
        ),
        (
          name: 'Huddle',
          type: 'stream',
          visibility: 'private',
          ttl: 3600,
          hidden: false,
        ),
      ];
      for (final c in cases) {
        final session = _FakeRelaySession(
          memberships: [
            _membership(_channelA, 'me'),
            _membership(_channelB, 'me'),
          ],
          metadata: [
            _meta(id: _channelA, name: 'general'),
            _meta(
              id: _channelB,
              name: c.name,
              channelType: c.type,
              visibility: c.visibility,
              ttlSeconds: c.ttl,
            ),
          ],
          // Old or inaccessible parent starts are absent from the bounded read.
          huddleStarts: const [],
        );
        final container = _buildContainer(session: session);
        try {
          final channels = await container.read(channelsProvider.future);
          expect(
            channels.any((channel) => channel.id == _channelB),
            !c.hidden,
            reason: '$c',
          );
          expect(
            container.read(channelViewerIsMemberProvider(_channelB)),
            isTrue,
          );
          await container.read(channelsProvider.notifier).refresh();
          expect(
            container
                .read(channelsProvider)
                .requireValue
                .any((channel) => channel.id == _channelB),
            !c.hidden,
            reason: '$c after refresh',
          );
        } finally {
          container.dispose();
        }
      }
    },
  );
}
