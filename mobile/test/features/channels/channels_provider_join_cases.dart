part of 'channels_provider_test.dart';

const _joinedChannel = '22222222-2222-4222-8222-222222222222';
const _otherJoinedChannel = '33333333-3333-4333-8333-333333333333';

void _confirmedJoinCases() {
  test('only a single member join_role lets a self-join post', () {
    final cases = <List<List<String>>, bool>{
      const []: false,
      const [
        ['join_role', 'member'],
      ]: false,
      const [
        ['join_role', 'guest'],
      ]: true,
      const [
        ['join_role'],
      ]: true,
      const [
        ['join_role', 'member', 'extra'],
      ]: true,
      const [
        ['join_role', 'member'],
        ['join_role', 'member'],
      ]: true,
    };
    cases.forEach((joinRoles, joinsAsGuest) {
      final base = _meta(id: _joinedChannel, name: 'joined');
      final event = NostrEvent(
        id: base.id,
        pubkey: base.pubkey,
        createdAt: base.createdAt,
        kind: base.kind,
        tags: [...base.tags, ...joinRoles],
        content: '',
        sig: 'sig',
      );
      expect(
        ChannelData.fromEvent(event).joinsAsGuest,
        joinsAsGuest,
        reason: '$joinRoles',
      );
    });
  });

  group('confirmed joins', () {
    test('a writer-confirmed removal retires the saved channel', () async {
      const scope = (communityId: 'community', pubkey: 'me');
      const page = (channelId: _joinedChannel, rootId: '');
      final session = _WriterReplicaRelaySession(
        memberships: [_membership(_joinedChannel, 'me')],
        metadata: [_meta(id: _joinedChannel, name: 'joined')],
      );
      final directory = await Directory.systemTemp.createTemp('buzz-lost-join');
      addTearDown(() => directory.delete(recursive: true));
      final purgeStarted = Completer<void>();
      final allowPurge = Completer<void>();
      final store = SavedMessageStore(
        root: () async {
          if (!purgeStarted.isCompleted) purgeStarted.complete();
          await allowPurge.future;
          return directory;
        },
        diagnostics: const NoopDiagnosticReporter(),
      );
      final container = _buildContainer(
        session: session,
        overrides: [
          savedMessageScopeProvider.overrideWithValue(scope),
          savedMessageStoreProvider.overrideWithValue(store),
        ],
      );
      addTearDown(container.dispose);
      await container.read(channelsProvider.future);
      final ticket = store.capture(scope, page);
      final retained = store.capture(scope, (
        channelId: _otherJoinedChannel,
        rootId: '',
      ));
      final notifier = container.read(channelsProvider.notifier);

      session.memberships = const [];
      session.writerMemberships = [_membership(_joinedChannel, 'me')];
      await notifier.refresh();
      expect(store.isCurrent(ticket), isTrue);

      session.writerMemberships = const [];
      var published = false;
      final refresh = notifier.refresh().then((_) => published = true);
      try {
        await purgeStarted.future;
        await _settle();
        expect(published, isFalse);
      } finally {
        allowPurge.complete();
        await refresh;
      }
      expect(store.isCurrent(ticket), isFalse);
      expect(store.isCurrent(retained), isTrue);
    });

    for (final noted in [true, false]) {
      test(
        noted
            ? 'a removal this client confirmed needs no writer read'
            : 'an unexplained removal asks the writer pool',
        () async {
          const scope = (communityId: 'community', pubkey: 'me');
          final session = _WriterReplicaRelaySession(
            memberships: [_membership(_joinedChannel, 'me')],
            metadata: [_meta(id: _joinedChannel, name: 'joined')],
          );
          final container = _buildContainer(
            session: session,
            overrides: [savedMessageScopeProvider.overrideWithValue(scope)],
          );
          addTearDown(container.dispose);
          await container.read(channelsProvider.future);
          final notifier = container.read(channelsProvider.notifier);

          if (noted) notifier.noteConfirmedRemoval(_joinedChannel);
          session.memberships = const [];
          await notifier.refresh();
          expect(session.strongMembershipReads, noted ? 0 : 1);
        },
      );
    }

    test(
      'a stale replica neither hides a confirmed join nor a later removal',
      () async {
        final session = _WriterReplicaRelaySession(
          memberships: const [],
          metadata: [
            _meta(id: _joinedChannel, name: 'joined'),
            _meta(id: _otherJoinedChannel, name: 'other'),
          ],
        );
        final container = _buildContainer(session: session);
        addTearDown(container.dispose);
        final notifier = container.read(channelsProvider.notifier);
        bool isMember(String id) =>
            container
                .read(channelsProvider)
                .value
                ?.any((channel) => channel.id == id && channel.isMember) ??
            false;

        await container.read(channelsProvider.future);
        session.writerMemberships = [_membership(_joinedChannel, 'me')];

        expect(await notifier.installConfirmedJoin(_joinedChannel), isTrue);
        expect(isMember(_joinedChannel), isTrue);

        // An ordinary refresh while the replica still lacks the join.
        await notifier.refresh();
        expect(isMember(_joinedChannel), isTrue);

        // The replica catches up; later reads stop asking the writer pool.
        session.memberships = [_membership(_joinedChannel, 'me')];
        await notifier.refresh();
        final strongReads = session.strongMembershipReads;
        await notifier.refresh();
        expect(session.strongMembershipReads, strongReads);
        expect(isMember(_joinedChannel), isTrue);

        // A join removed on the writer before replicas show it is not kept.
        session.writerMemberships = [
          _membership(_joinedChannel, 'me'),
          _membership(_otherJoinedChannel, 'me'),
        ];
        expect(
          await notifier.installConfirmedJoin(_otherJoinedChannel),
          isTrue,
        );
        session.writerMemberships = [_membership(_joinedChannel, 'me')];
        await notifier.refresh();
        expect(isMember(_otherJoinedChannel), isFalse);
        final afterRemoval = session.strongMembershipReads;
        await notifier.refresh();
        expect(session.strongMembershipReads, afterRemoval);

        // A join recorded for one identity never pins another's reads.
        session.writerMemberships = [
          _membership(_joinedChannel, 'me'),
          _membership(_otherJoinedChannel, 'me'),
        ];
        expect(
          await notifier.installConfirmedJoin(_otherJoinedChannel),
          isTrue,
        );
        final beforeSwitch = session.strongMembershipReads;
        container.read(_testPubkeyProvider.notifier).set('someone-else');
        await container.read(channelsProvider.future);
        await notifier.refresh();
        expect(session.strongMembershipReads, beforeSwitch);
      },
    );

    test(
      'join settles only once the channel list shows the membership',
      () async {
        final keys = nostr.Keys.generate();
        final session = _WriterReplicaRelaySession(
          memberships: const [],
          metadata: [_meta(id: _joinedChannel, name: 'joined')],
        );
        final container = _buildContainer(session: session);
        addTearDown(container.dispose);
        final actionsProvider = Provider<ChannelActions>(
          (ref) => ChannelActions(
            ref: ref,
            session: session,
            signedEventRelay: SignedEventRelay(
              session: session,
              nsec: keys.nsec,
            ),
            currentPubkey: 'me',
          ),
        );
        bool isMember() =>
            container
                .read(channelsProvider)
                .value
                ?.any(
                  (channel) => channel.id == _joinedChannel && channel.isMember,
                ) ??
            false;
        await container.read(channelsProvider.future);
        final actions = container.read(actionsProvider);

        // Accepted, but the roster read fails: not a rejection.
        session.writerMemberships = [_membership(_joinedChannel, 'me')];
        session.rosterFailures = 1;
        await expectLater(
          actions.joinChannel(_joinedChannel),
          throwsA(isA<ChannelJoinUnconfirmed>()),
        );
        expect(session.published.map((event) => event.kind), [9021]);
        expect(isMember(), isFalse);

        // Confirmed, but the list can't show the channel (no metadata yet).
        final metadata = session.metadata;
        session.metadata = const [];
        await expectLater(
          actions.recheckJoin(_joinedChannel),
          throwsA(isA<ChannelJoinUnconfirmed>()),
        );
        expect(isMember(), isFalse);
        session.metadata = metadata;

        // Confirmed, but the list read fails: not a rejection either.
        session.membershipFailures = 1;
        await expectLater(
          actions.recheckJoin(_joinedChannel),
          throwsA(isA<ChannelJoinUnconfirmed>()),
        );
        expect(isMember(), isFalse);

        // Checking again shows the membership without sending the join again.
        await actions.recheckJoin(_joinedChannel);
        expect(isMember(), isTrue);
        expect(session.published, hasLength(1));
      },
    );

    test(
      'an add that lands in another DM is confirmed through the writer',
      () async {
        final keys = nostr.Keys.generate();
        final session = _WriterReplicaRelaySession(
          memberships: const [],
          metadata: [
            _meta(id: _otherJoinedChannel, name: 'dm', channelType: 'dm'),
            _meta(id: _joinedChannel, name: 'new dm', channelType: 'dm'),
          ],
        )..dmAddResponse = 'response:{"channel_id":"$_joinedChannel"}';
        final container = _buildContainer(session: session);
        addTearDown(container.dispose);
        final actions = container.read(
          Provider<ChannelActions>(
            (ref) => ChannelActions(
              ref: ref,
              session: session,
              signedEventRelay: SignedEventRelay(
                session: session,
                nsec: keys.nsec,
              ),
              currentPubkey: 'me',
            ),
          ),
        );
        await container.read(channelsProvider.future);

        // The writer has the new DM; the replica the list reads does not.
        session.writerMemberships = [_membership(_joinedChannel, 'me')];
        session.rosterFailures = 1;
        await expectLater(
          actions.addToDm(
            channelId: _otherJoinedChannel,
            pubkeys: const ['robin'],
            history: DmHistory.startNew,
          ),
          throwsA(
            isA<DmAddUnconfirmed>().having(
              (error) => error.channelId,
              'channelId',
              _joinedChannel,
            ),
          ),
        );

        // Opening it again reads the writer and sends nothing.
        final dm = await actions.openAddedDm(_joinedChannel);
        expect(dm.id, _joinedChannel);
        expect(session.published.map((event) => event.kind), [41011]);
      },
    );

    test(
      'a DM add asks the relay, and its refusals become words to show',
      () async {
        final keys = nostr.Keys.generate();
        final session = _WriterReplicaRelaySession(memberships: const [])
          ..dmAddRefusals.addAll([
            'needs-choice: history',
            'invalid: history must be "include", got "ask"',
            'invalid: DM supports at most 9 people',
            'invalid: a DM may hold at most 9 people; agents do not count',
            'needs-choice: history',
            'restricted: you cannot add people to this DM',
          ]);
        final container = _buildContainer(session: session);
        addTearDown(container.dispose);
        final actions = container.read(
          Provider<ChannelActions>(
            (ref) => ChannelActions(
              ref: ref,
              session: session,
              signedEventRelay: SignedEventRelay(
                session: session,
                nsec: keys.nsec,
              ),
              currentPubkey: 'me',
            ),
          ),
        );
        Future<void> add(DmHistory history) => actions.addToDm(
          channelId: _otherJoinedChannel,
          pubkeys: const ['robin'],
          history: history,
        );
        Matcher refused(String message) => throwsA(
          isA<Exception>().having(
            (error) => error.toString(),
            'message',
            'Exception: $message',
          ),
        );

        // The relay asks, and so does one from before it knew "ask".
        await expectLater(
          add(DmHistory.ask),
          throwsA(isA<DmNeedsHistoryChoice>()),
        );
        await expectLater(
          add(DmHistory.ask),
          throwsA(isA<DmNeedsHistoryChoice>()),
        );
        await expectLater(add(DmHistory.ask), refused(dmAdditionFailedMessage));
        await expectLater(
          add(DmHistory.include),
          refused(dmAdditionFailedMessage),
        );
        // Only an add that asked can be told to choose.
        await expectLater(
          add(DmHistory.include),
          refused('Couldn’t add them. Try again.'),
        );
        await expectLater(
          add(DmHistory.startNew),
          refused('Couldn’t add them. Try again.'),
        );
        expect(
          session.published.map(
            (event) =>
                event.tags.where((tag) => tag.first == 'history').toList(),
          ),
          [
            [
              ['history', 'ask'],
            ],
            [
              ['history', 'ask'],
            ],
            [
              ['history', 'ask'],
            ],
            [
              ['history', 'include'],
            ],
            [
              ['history', 'include'],
            ],
            <List<String>>[],
          ],
        );
      },
    );

    test(
      'an agent whose history tag is refused adds again without it',
      () async {
        final keys = nostr.Keys.generate();
        final session =
            _WriterReplicaRelaySession(
                memberships: const [],
                metadata: [
                  _meta(id: _otherJoinedChannel, name: 'dm', channelType: 'dm'),
                  _meta(id: _joinedChannel, name: 'new dm', channelType: 'dm'),
                ],
              )
              ..dmAddResponse = 'response:{"channel_id":"$_joinedChannel"}'
              ..writerMemberships = [_membership(_joinedChannel, 'me')];
        final container = _buildContainer(session: session);
        addTearDown(container.dispose);
        final actions = container.read(
          Provider<ChannelActions>(
            (ref) => ChannelActions(
              ref: ref,
              session: session,
              signedEventRelay: SignedEventRelay(
                session: session,
                nsec: keys.nsec,
              ),
              currentPubkey: 'me',
            ),
          ),
        );
        await container.read(channelsProvider.future);

        for (final history in [DmHistory.ask, DmHistory.include]) {
          session.dmAddRefusals.add(
            "forbidden: only people can share a DM's history",
          );
          final dm = await actions.addToDm(
            channelId: _otherJoinedChannel,
            pubkeys: const ['robin'],
            history: history,
          );
          expect(dm.id, _joinedChannel);
        }
        expect(
          session.published.map(
            (event) =>
                event.tags.where((tag) => tag.first == 'history').toList(),
          ),
          [
            [
              ['history', 'ask'],
            ],
            <List<String>>[],
            [
              ['history', 'include'],
            ],
            <List<String>>[],
          ],
        );
      },
    );

    test('an agent policy refusal of a DM open or add names the agent', () async {
      final keys = nostr.Keys.generate();
      final scout = 'd' * 64;
      final unknown = 'e' * 64;
      final session = _WriterReplicaRelaySession(memberships: const [])
        ..dmOpenRefusals.addAll([
          'forbidden: policy:owner_only — only the agent owner can add this agent (agent $scout)',
          'forbidden: policy:owner_only — only the agent owner can add this agent (agent $unknown)',
          'restricted: no',
        ])
        ..dmAddRefusals.addAll([
          'forbidden: policy:nobody — this agent has disabled external channel additions (agent $scout)',
          'forbidden: policy:nobody — this agent has disabled external channel additions',
        ]);
      final container = _buildContainer(session: session);
      addTearDown(container.dispose);
      container.read(_testPubkeyProvider.notifier).set('9' * 64);
      container
          .read(userCacheProvider.notifier)
          .put(UserProfile(pubkey: scout, displayName: 'Scout'));
      final actions = container.read(
        Provider<ChannelActions>(
          (ref) => ChannelActions(
            ref: ref,
            session: session,
            signedEventRelay: SignedEventRelay(
              session: session,
              nsec: keys.nsec,
            ),
            currentPubkey: '9' * 64,
          ),
        ),
      );
      Matcher shows(String message) => throwsA(
        isA<DmAgentRefused>().having(
          (error) => error.toString(),
          'message',
          message,
        ),
      );
      Future<void> open() =>
          actions.openDm(pubkeys: [scout], includeHistory: true);
      Future<void> add() => actions.addToDm(
        channelId: _otherJoinedChannel,
        pubkeys: [scout],
        history: DmHistory.ask,
      );

      await expectLater(
        open(),
        shows('Only the owner of Scout can add it to a DM.'),
      );
      // Without a name to show, it says whose owner in general.
      await expectLater(
        open(),
        shows('Only the owner of this agent can add it to a DM.'),
      );
      // Any other refusal is shown as before.
      await expectLater(
        open(),
        throwsA(
          isA<Exception>()
              .having((e) => e is DmAgentRefused, 'named', isFalse)
              .having(
                (e) => e.toString(),
                'message',
                'Exception: restricted: no',
              ),
        ),
      );
      await expectLater(
        add(),
        shows('Couldn’t add them. Scout can’t be added to a DM.'),
      );
      // A relay that names no agent refuses as before.
      await expectLater(
        add(),
        throwsA(
          isA<Exception>().having(
            (error) => error.toString(),
            'message',
            'Exception: Couldn’t add them. Try again.',
          ),
        ),
      );
    });

    test('a refused agent is named by its disambiguated label', () async {
      final keys = nostr.Keys.generate();
      final scout = 'd' * 64;
      final otherScout = 'c' * 64;
      final alice = 'a' * 64;
      final aliceAgent = 'b' * 64;
      final morgan = '1' * 64;
      final session = _WriterReplicaRelaySession(memberships: const []);
      final container = _buildContainer(session: session);
      addTearDown(container.dispose);
      container.read(_testPubkeyProvider.notifier).set('9' * 64);
      container.read(userCacheProvider.notifier)
        ..put(UserProfile(pubkey: morgan, displayName: 'Morgan'))
        ..put(UserProfile(pubkey: alice, displayName: 'Alice'))
        ..put(
          UserProfile(pubkey: scout, displayName: 'Scout', ownerPubkey: morgan),
        )
        ..put(
          UserProfile(
            pubkey: otherScout,
            displayName: 'Scout',
            ownerPubkey: alice,
          ),
        )
        ..put(UserProfile(pubkey: aliceAgent, displayName: 'Alice'));
      final actions = container.read(
        Provider<ChannelActions>(
          (ref) => ChannelActions(
            ref: ref,
            session: session,
            signedEventRelay: SignedEventRelay(
              session: session,
              nsec: keys.nsec,
            ),
            currentPubkey: '9' * 64,
          ),
        ),
      );
      Future<void> open(String agent, List<String> others, bool ownerOnly) {
        session.dmOpenRefusals.add(
          'forbidden: policy:${ownerOnly ? 'owner_only' : 'nobody'} — no '
          '(agent $agent)',
        );
        return actions.openDm(pubkeys: [agent, ...others]);
      }

      Matcher shows(String message) => throwsA(
        isA<DmAgentRefused>().having(
          (error) => error.toString(),
          'message',
          message,
        ),
      );

      // Two agents named Scout differ by their owners, and the owner is
      // named too.
      await expectLater(
        open(scout, [otherScout], true),
        shows('Only Morgan can add Morgan’s Scout to a DM.'),
      );
      await expectLater(
        open(otherScout, [scout], false),
        shows('Alice’s Scout can’t be added to a DM.'),
      );
      // An agent sharing a person's name is marked as the agent.
      await expectLater(
        open(aliceAgent, [alice], true),
        shows('Only the owner of Alice (agent) can add it to a DM.'),
      );
      // A unique name stays plain, and an owner with no profile or no
      // verified owner is not named.
      await expectLater(
        open(scout, [], true),
        shows('Only Morgan can add Scout to a DM.'),
      );
      container
          .read(userCacheProvider.notifier)
          .put(UserProfile(pubkey: scout, displayName: 'Scout'));
      await expectLater(
        open(scout, [], true),
        shows('Only the owner of Scout can add it to a DM.'),
      );

      // A refusal waits only for the names its sentence uses. Naming a
      // cached, uniquely named agent reads no profile, though its owner,
      // the DM's other people and this account are uncached; a refusal that
      // names who may add the agent still reads its owner.
      final stranger = '7' * 64;
      final unloadedOwner = '8' * 64;
      container
          .read(userCacheProvider.notifier)
          .put(
            UserProfile(
              pubkey: scout,
              displayName: 'Scout',
              ownerPubkey: unloadedOwner,
            ),
          );
      List<List<String>?> profileReads() => [
        for (final filter in session.historyFilters)
          if (filter.kinds.contains(0)) filter.authors,
      ];
      session.historyFilters.clear();
      await expectLater(
        open(scout, [stranger], false),
        shows('Scout can’t be added to a DM.'),
      );
      expect(profileReads(), isEmpty);
      await expectLater(
        open(scout, [stranger], true),
        shows('Only the owner of Scout can add it to a DM.'),
      );
      expect(profileReads(), [
        [unloadedOwner],
      ]);
    });

    group('a refused agent added to a DM', () {
      const dmId = 'dm-ahead';
      final me = '9' * 64, scout = 'd' * 64, otherScout = 'c' * 64;
      final morgan = '1' * 64, alice = 'a' * 64;
      late _WriterReplicaRelaySession session;
      late ProviderContainer container;
      late ChannelActions actions;
      setUp(() {
        session = _WriterReplicaRelaySession(memberships: const []);
        container = _buildContainer(session: session);
        container.read(_testPubkeyProvider.notifier).set(me);
        actions = container.read(
          Provider<ChannelActions>(
            (ref) => ChannelActions(
              ref: ref,
              session: session,
              signedEventRelay: SignedEventRelay(
                session: session,
                nsec: nostr.Keys.generate().nsec,
              ),
              currentPubkey: me,
            ),
          ),
        );
        // Opened from New Message before the channel list has it.
        container
            .read(channelsProvider.notifier)
            .rememberOpened(
              Channel(
                id: dmId,
                name: 'DM',
                channelType: 'dm',
                visibility: 'private',
                description: '',
                createdBy: me,
                createdAt: DateTime(2025),
                memberCount: 2,
                participantPubkeys: [me, otherScout],
                isMember: true,
              ),
            );
      });
      tearDown(() => container.dispose());

      Future<void> add() {
        session.dmAddRefusals.add(
          'forbidden: policy:owner_only — no (agent $scout)',
        );
        return actions.addToDm(
          channelId: dmId,
          pubkeys: [scout],
          history: DmHistory.include,
        );
      }

      test('compares against a DM opened ahead of the list', () async {
        container.read(userCacheProvider.notifier)
          ..put(UserProfile(pubkey: morgan, displayName: 'Morgan'))
          ..put(UserProfile(pubkey: alice, displayName: 'Alice'))
          ..put(
            UserProfile(
              pubkey: scout,
              displayName: 'Scout',
              ownerPubkey: morgan,
            ),
          )
          ..put(
            UserProfile(
              pubkey: otherScout,
              displayName: 'Scout',
              ownerPubkey: alice,
            ),
          );
        await expectLater(
          add(),
          throwsA(
            isA<DmAgentRefused>().having(
              (error) => error.toString(),
              'message',
              'Couldn’t add them. Only Morgan can add Morgan’s Scout to a DM.',
            ),
          ),
        );
      });

      test('loads every agent owner in one read', () async {
        container.read(userCacheProvider.notifier)
          ..put(UserProfile(pubkey: me, displayName: 'Me'))
          ..put(
            UserProfile(
              pubkey: scout,
              displayName: 'Scout',
              ownerPubkey: morgan,
            ),
          )
          ..put(
            UserProfile(
              pubkey: otherScout,
              displayName: 'Scout',
              ownerPubkey: alice,
            ),
          );
        await expectLater(add(), throwsA(isA<DmAgentRefused>()));
        expect(session.profileReads, [
          unorderedEquals([morgan, alice]),
        ]);
      });

      test('tells two owners named Morgan apart', () async {
        container.read(userCacheProvider.notifier)
          ..put(UserProfile(pubkey: morgan, displayName: 'Morgan'))
          ..put(UserProfile(pubkey: alice, displayName: 'Morgan'))
          ..put(
            UserProfile(
              pubkey: scout,
              displayName: 'Scout',
              ownerPubkey: morgan,
            ),
          )
          ..put(
            UserProfile(
              pubkey: otherScout,
              displayName: 'Scout',
              ownerPubkey: alice,
            ),
          );
        await expectLater(
          add(),
          throwsA(
            isA<DmAgentRefused>().having(
              (error) => error.toString(),
              'message',
              'Couldn’t add them. Only Morgan · sl3h can add Morgan’s Scout · '
                  'dmpv to a DM.',
            ),
          ),
        );
      });

      for (final ownerOnly in [false, true]) {
        test('waits for an owner already in the DM before naming '
            '(${ownerOnly ? 'owner_only' : 'nobody'})', () async {
          // Bob adds Morgan's Scout to a DM with Morgan, not yet loaded, and
          // Alice's Scout.
          const withMorgan = 'dm-morgan';
          container
              .read(channelsProvider.notifier)
              .rememberOpened(
                Channel(
                  id: withMorgan,
                  name: 'DM',
                  channelType: 'dm',
                  visibility: 'private',
                  description: '',
                  createdBy: me,
                  createdAt: DateTime(2025),
                  memberCount: 3,
                  participantPubkeys: [me, morgan, otherScout],
                  isMember: true,
                ),
              );
          final cache = container.read(userCacheProvider.notifier)
            ..put(UserProfile(pubkey: alice, displayName: 'Alice'))
            ..put(
              UserProfile(
                pubkey: scout,
                displayName: 'Scout',
                ownerPubkey: morgan,
              ),
            )
            ..put(
              UserProfile(
                pubkey: otherScout,
                displayName: 'Scout',
                ownerPubkey: alice,
              ),
            );
          session.onProfileRead = (authors) {
            if (authors.contains(morgan)) {
              cache.put(UserProfile(pubkey: morgan, displayName: 'Morgan'));
            }
          };
          await expectLater(
            () {
              session.dmAddRefusals.add(
                'forbidden: policy:${ownerOnly ? 'owner_only' : 'nobody'} — '
                'no (agent $scout)',
              );
              return actions.addToDm(
                channelId: withMorgan,
                pubkeys: [scout],
                history: DmHistory.include,
              );
            }(),
            throwsA(
              isA<DmAgentRefused>().having(
                (error) => error.toString(),
                'message',
                ownerOnly
                    ? 'Couldn’t add them. Only Morgan can add Morgan’s Scout '
                          'to a DM.'
                    : 'Couldn’t add them. Morgan’s Scout can’t be added to a DM.',
              ),
            ),
          );
        });
      }

      test('names with what it has once profiles run late', () async {
        session.profileReadsHang = true;
        final clock = Stopwatch()..start();
        await expectLater(
          add(),
          throwsA(
            isA<DmAgentRefused>().having(
              (error) => error.toString(),
              'message',
              startsWith('Couldn’t add them. Only the owner of '),
            ),
          ),
        );
        expect(clock.elapsed, lessThan(const Duration(seconds: 4)));
      });
    });

    test('a list rebuild during the join still shows the membership', () async {
      final session = _WriterReplicaRelaySession(
        memberships: const [],
        metadata: [_meta(id: _joinedChannel, name: 'joined')],
      );
      final container = _buildContainer(session: session);
      addTearDown(container.dispose);
      await container.read(channelsProvider.future);
      session.writerMemberships = [_membership(_joinedChannel, 'me')];

      final gate = Completer<void>();
      session.metadataGate = gate;
      final joined = container
          .read(channelsProvider.notifier)
          .installConfirmedJoin(_joinedChannel);
      await _waitUntil(() => session.strongMembershipReads > 0);
      container.invalidate(channelsProvider);
      session.metadataGate = null;
      gate.complete();

      expect(await joined, isTrue);
    });
  });
}

/// A relay whose writer pool sees joins before its read replicas do.
///
/// Only reads that ask for `consistency: strong` reach [writerMemberships].
class _WriterReplicaRelaySession extends _FakeRelaySession {
  _WriterReplicaRelaySession({required super.memberships, super.metadata});

  List<NostrEvent> writerMemberships = const [];
  int strongMembershipReads = 0;
  int rosterFailures = 0;
  final published = <NostrEvent>[];

  /// The relay's answer to a DM add (kind:41011), as its OK message.
  String? dmAddResponse;

  /// The relay's refusals of the next DM adds, in order, as their OK messages.
  final dmAddRefusals = <String>[];

  /// The relay's refusals of the next DM opens, in order, as their OK messages.
  final dmOpenRefusals = <String>[];

  /// The authors of each profile read, in order.
  final profileReads = <List<String>>[];

  /// Whether profile reads never answer.
  bool profileReadsHang = false;

  /// Called with each profile read's authors, before it answers.
  void Function(List<String> authors)? onProfileRead;

  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) {
    if (filter.kinds case [0]) {
      profileReads.add([...?filter.authors]);
      onProfileRead?.call([...?filter.authors]);
      if (profileReadsHang) return Completer<List<NostrEvent>>().future;
    }
    return super.fetchHistory(filter, timeout: timeout);
  }

  @override
  Future<NostrEvent> publish(
    NostrEvent event, {
    Duration timeout = const Duration(seconds: 8),
    Future<bool> Function()? mayPublish,
  }) async {
    published.add(event);
    if (event.kind == 41011 && dmAddRefusals.isNotEmpty) {
      throw Exception(dmAddRefusals.removeAt(0));
    }
    if (event.kind == 41010 && dmOpenRefusals.isNotEmpty) {
      throw Exception(dmOpenRefusals.removeAt(0));
    }
    final response = dmAddResponse;
    if (event.kind == 41011 && response != null) {
      return NostrEvent(
        id: event.id,
        pubkey: event.pubkey,
        createdAt: event.createdAt,
        kind: event.kind,
        tags: event.tags,
        content: response,
        sig: event.sig,
      );
    }
    return event;
  }

  @override
  Future<List<NostrEvent>> queryRelay(
    List<NostrFilter> filters, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    if (filters case [final filter]
        when filter.kinds.length == 1 &&
            filter.kinds.single == 39002 &&
            filter.extensions['consistency'] == 'strong') {
      final channelIds = filter.tags['#d']?.toSet();
      if (channelIds != null) {
        if (rosterFailures > 0) {
          rosterFailures--;
          throw Exception('roster read failed');
        }
        return writerMemberships
            .where((event) => channelIds.contains(event.getTagValue('d')))
            .toList();
      }
      strongMembershipReads++;
      if (membershipFailures > 0) {
        membershipFailures--;
        throw Exception('membership fetch failed');
      }
      if (filter.until != null) return const [];
      return List.of(writerMemberships);
    }
    return super.queryRelay(filters, timeout: timeout);
  }
}
