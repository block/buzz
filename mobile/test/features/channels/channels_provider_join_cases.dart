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

  @override
  Future<NostrEvent> publish(
    NostrEvent event, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    published.add(event);
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
