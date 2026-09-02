import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:buzz/features/channels/channel_management_provider.dart';
import 'package:buzz/features/channels/mobile_huddle_controller.dart';
import 'package:buzz/shared/relay/relay.dart';

/// Tests for [channelDetailsFromEvent].
///
/// The function maps a kind:39000 metadata event to [ChannelDetails], and is
/// the source of truth for the merge that [Channel.mergeDetails] performs in
/// the channel detail view. Anything `ChannelData.fromEvent` parses that's
/// also exposed on `ChannelDetails` MUST be propagated here — otherwise
/// `mergeDetails` silently clears that state on the merged Channel.
void main() {
  test('extracts unique relay members from current and legacy tags', () {
    final pubkeys = relayMemberPubkeysFromEvents([
      NostrEvent(
        id: 'members-1',
        pubkey: 'relay',
        createdAt: 1700000000,
        kind: 13534,
        tags: const [
          ['member', 'ALICE', 'member'],
          ['member', 'bob', 'admin'],
          ['p', 'alice', '', 'member'],
          ['name', 'not-a-member'],
          ['member', ''],
        ],
        content: '',
        sig: 'sig',
      ),
    ]);

    expect(pubkeys, ['alice', 'bob']);
  });

  test('builds an alphabetized directory from the latest profile events', () {
    final users = directoryUsersFromProfileEvents([
      NostrEvent(
        id: 'alice-old',
        pubkey: 'alice',
        createdAt: 10,
        kind: 0,
        tags: const [],
        content: '{"display_name":"Zoe"}',
        sig: 'sig',
      ),
      NostrEvent(
        id: 'bob',
        pubkey: 'bob',
        createdAt: 20,
        kind: 0,
        tags: const [],
        content: '{"display_name":"Bob"}',
        sig: 'sig',
      ),
      NostrEvent(
        id: 'alice-new',
        pubkey: 'ALICE',
        createdAt: 30,
        kind: 0,
        tags: const [],
        content:
            '{"display_name":"Alice","picture":"https://example.com/alice.png"}',
        sig: 'sig',
      ),
      NostrEvent(
        id: 'not-a-profile',
        pubkey: 'charlie',
        createdAt: 40,
        kind: 1,
        tags: const [],
        content: '{}',
        sig: 'sig',
      ),
    ]);

    expect(users.map((user) => user.label), ['Alice', 'Bob']);
    expect(users.first.pubkey, 'alice');
    expect(users.first.avatarUrl, 'https://example.com/alice.png');
  });

  test('propagates archived state from kind:39000 archived tag', () {
    // Regression: previously this mapping ignored the `archived` tag, so
    // `Channel.mergeDetails` would clear the archived flag the list provider
    // had set, and the detail screen would show compose/manage actions for
    // expired/archived TTL channels.
    final details = channelDetailsFromEvent(
      NostrEvent(
        id: 'meta-1',
        pubkey: 'creator',
        createdAt: 1700000000,
        kind: 39000,
        tags: const [
          ['d', 'c8c629ae-d35c-44fa-bc39-f6c1816756cc'],
          ['name', 'expired-ttl'],
          ['t', 'stream'],
          ['public'],
          ['ttl', '86400'],
          ['archived', 'true'],
        ],
        content: '',
        sig: 'sig',
      ),
    );

    expect(details.archivedAt, isNotNull);
    expect(details.ttlSeconds, 86400);
  });

  test('omits archivedAt when no archived tag is present', () {
    final details = channelDetailsFromEvent(
      NostrEvent(
        id: 'meta-1',
        pubkey: 'creator',
        createdAt: 1700000000,
        kind: 39000,
        tags: const [
          ['d', 'c8c629ae-d35c-44fa-bc39-f6c1816756cc'],
          ['name', 'active'],
          ['t', 'stream'],
          ['public'],
        ],
        content: '',
        sig: 'sig',
      ),
    );

    expect(details.archivedAt, isNull);
    expect(details.ttlSeconds, isNull);
  });

  test('propagates ttl_deadline tag', () {
    final details = channelDetailsFromEvent(
      NostrEvent(
        id: 'meta-1',
        pubkey: 'creator',
        createdAt: 1700000000,
        kind: 39000,
        tags: const [
          ['d', 'c8c629ae-d35c-44fa-bc39-f6c1816756cc'],
          ['name', 'with-deadline'],
          ['t', 'stream'],
          ['public'],
          ['ttl', '86400'],
          ['ttl_deadline', '2026-05-14T19:54:06.989151+00:00'],
        ],
        content: '',
        sig: 'sig',
      ),
    );

    expect(details.ttlSeconds, 86400);
    expect(details.ttlDeadline, isNotNull);
    expect(details.ttlDeadline!.isUtc, isTrue);
  });

  group('buildCreateChannelTags', () {
    test('builds an ongoing channel without a ttl tag', () {
      final tags = buildCreateChannelTags(
        channelId: 'c8c629ae-d35c-44fa-bc39-f6c1816756cc',
        name: 'release-notes',
        channelType: 'stream',
        visibility: 'open',
        description: '  Ship updates  ',
      );

      expect(tags, [
        ['h', 'c8c629ae-d35c-44fa-bc39-f6c1816756cc'],
        ['name', 'release-notes'],
        ['visibility', 'open'],
        ['channel_type', 'stream'],
        ['about', 'Ship updates'],
      ]);
    });

    test('adds the selected ttl for a temporary channel', () {
      final tags = buildCreateChannelTags(
        channelId: 'c8c629ae-d35c-44fa-bc39-f6c1816756cc',
        name: 'incident-room',
        channelType: 'stream',
        visibility: 'private',
        ttlSeconds: 604800,
      );

      expect(tags, contains(equals(['ttl', '604800'])));
    });
  });

  group('buildDeleteMessageTags', () {
    test('emits both channel h tag and target e tag', () {
      final tags = buildDeleteMessageTags(
        channelId: 'c8c629ae-d35c-44fa-bc39-f6c1816756cc',
        eventId: 'abc123',
      );

      expect(tags, [
        ['h', 'c8c629ae-d35c-44fa-bc39-f6c1816756cc'],
        ['e', 'abc123'],
      ]);
    });
  });

  group('build channel lifecycle tags', () {
    test('channel edits match desktop kind 9002 tags', () {
      expect(
        buildUpdateChannelTags(
          channelId: 'channel-id',
          name: '  ###general  ',
          description: ' Team updates ',
        ),
        [
          ['h', 'channel-id'],
          ['name', 'general'],
          ['about', 'Team updates'],
        ],
      );
    });

    test('channel edits reject a hash-only name', () {
      expect(
        () => buildUpdateChannelTags(channelId: 'channel-id', name: ' ### '),
        throwsArgumentError,
      );
    });

    test('archive matches kind 9002 tags', () {
      expect(buildSetChannelArchivedTags('channel-id', archived: true), [
        ['h', 'channel-id'],
        ['archived', 'true'],
      ]);
    });

    test('unarchive matches kind 9002 tags', () {
      expect(buildSetChannelArchivedTags('channel-id', archived: false), [
        ['h', 'channel-id'],
        ['archived', 'false'],
      ]);
    });

    test('delete matches desktop kind 9008 tags', () {
      expect(buildDeleteChannelTags('channel-id'), [
        ['h', 'channel-id'],
      ]);
    });
  });

  test(
    'create and join stop before submitting after a community switch',
    () async {
      final keys = nostr.Keys.generate();
      final session = _RecordingPublishRelaySession();
      final actionsProvider = Provider<ChannelActions>((ref) {
        return ChannelActions(
          ref: ref,
          session: session,
          signedEventRelay: SignedEventRelay(session: session, nsec: keys.nsec),
          currentPubkey: keys.public,
          isCommunityValid: () => false,
        );
      });
      final container = ProviderContainer(retry: (_, _) => null);
      addTearDown(container.dispose);

      final actions = container.read(actionsProvider);
      await expectLater(
        actions.createChannel(
          channelId: _channelId,
          name: 'general',
          channelType: 'stream',
          visibility: 'open',
        ),
        throwsA(isA<StateError>()),
      );
      await expectLater(
        actions.joinChannel(_channelId),
        throwsA(isA<StateError>()),
      );

      expect(session.publishedEvents, isEmpty);
    },
  );

  group('Huddle channel lifecycle', () {
    test(
      'starts from accepted signed events without refreshing all channels',
      () async {
        final keys = nostr.Keys.generate();
        final session = _RecordingPublishRelaySession();
        final actionsProvider = Provider<ChannelActions>((ref) {
          return ChannelActions(
            ref: ref,
            session: session,
            signedEventRelay: SignedEventRelay(
              session: session,
              nsec: keys.nsec,
            ),
            currentPubkey: keys.public,
          );
        });
        final container = ProviderContainer(
          retry: (_, _) => null,
          overrides: [
            relaySessionProvider.overrideWith(() => session),
            myPubkeyProvider.overrideWithValue(keys.public),
          ],
        );
        addTearDown(container.dispose);

        final actions = container.read(actionsProvider);
        final backingChannelId = await actions.createHuddleBackingChannel();
        final started = await actions.announceHuddleStarted(
          parentChannelId: _channelId,
          ephemeralChannelId: backingChannelId,
        );

        expect(session.historyQueryCount, 0);
        expect(session.publishedEvents, hasLength(2));
        final create = session.publishedEvents.first;
        expect(create.kind, 9007);
        expect(create.tags, contains(equals(['h', backingChannelId])));
        expect(create.tags, contains(equals(['visibility', 'private'])));
        expect(create.tags, contains(equals(['channel_type', 'stream'])));
        expect(create.tags, contains(equals(['ttl', '3600'])));

        final start = session.publishedEvents.last;
        expect(start.kind, EventKind.huddleStarted);
        expect(start.tags, [
          ['h', _channelId],
        ]);
        expect(start.content, '{"ephemeral_channel_id":"$backingChannelId"}');
        expect(started.id, start.id);
      },
    );

    test(
      'publishes ephemeral Huddle reactions on the backing channel',
      () async {
        final keys = nostr.Keys.generate();
        final session = _RecordingPublishRelaySession();
        final actionsProvider = Provider<ChannelActions>((ref) {
          return ChannelActions(
            ref: ref,
            session: session,
            signedEventRelay: SignedEventRelay(
              session: session,
              nsec: keys.nsec,
            ),
            currentPubkey: keys.public,
          );
        });
        final container = ProviderContainer(
          retry: (_, _) => null,
          overrides: [
            relaySessionProvider.overrideWith(() => session),
            myPubkeyProvider.overrideWithValue(keys.public),
          ],
        );
        addTearDown(container.dispose);

        await container
            .read(actionsProvider)
            .sendHuddleReaction(
              channelId: _channelId,
              emoji: ' 🎉 ',
              senderName: 'Self',
            );

        expect(session.publishedEvents, hasLength(1));
        final reaction = session.publishedEvents.single;
        expect(reaction.kind, EventKind.huddleReaction);
        expect(reaction.content, '🎉');
        expect(reaction.tags, [
          ['h', _channelId],
          ['reaction', '🎉'],
          ['sender_name', 'Self'],
        ]);
      },
    );

    test('last-human count ignores a stale cached roster', () async {
      final session = _ConnectionAwareRelaySession();
      final container = ProviderContainer(
        retry: (_, _) => null,
        overrides: [
          relaySessionProvider.overrideWith(() => session),
          channelMembersProvider(_channelId).overrideWith(
            (ref) async => [
              ChannelMember(
                pubkey: _memberPubkey,
                role: 'admin',
                joinedAt: DateTime(2025),
              ),
              ChannelMember(
                pubkey:
                    'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb',
                role: 'member',
                joinedAt: DateTime(2025),
              ),
            ],
          ),
        ],
      );
      addTearDown(container.dispose);
      container.read(relaySessionProvider);
      session.connect();
      expect(
        await container.read(channelMembersProvider(_channelId).future),
        hasLength(2),
      );

      final humanCount = await container.read(huddleHumanCountProvider)(
        _channelId,
      );

      expect(humanCount, 1);
      expect(session.historyQueryCount, 1);
    });
  });

  test(
    'stale channel actions cannot publish after a community switch',
    () async {
      final session = RelaySessionNotifier();
      final container = ProviderContainer(
        retry: (_, _) => null,
        overrides: [
          relayConfigProvider.overrideWith(_FixedRelayConfigNotifier.new),
          relaySessionProvider.overrideWith(() => session),
        ],
      );
      addTearDown(container.dispose);

      final staleActions = container.read(channelActionsProvider);
      container
          .read(relayConfigProvider.notifier)
          .update(baseUrl: 'https://other-community.example', nsec: null);

      final operations = <Future<void> Function()>[
        () =>
            staleActions.updateChannel(channelId: _channelId, name: 'renamed'),
        () => staleActions.archiveChannel(_channelId),
        () => staleActions.unarchiveChannel(_channelId),
        () => staleActions.deleteChannel(_channelId),
        () => staleActions.setCanvas(channelId: _channelId, content: 'canvas'),
        () => staleActions.changeMemberRole(
          channelId: _channelId,
          pubkey: _memberPubkey,
          role: 'admin',
        ),
        () => staleActions.removeMember(
          channelId: _channelId,
          pubkey: _memberPubkey,
        ),
        () => staleActions.leaveChannel(_channelId),
      ];

      for (final operation in operations) {
        await expectLater(operation(), throwsA(isA<StateError>()));
      }
    },
  );

  group('channelMembersProvider', () {
    test('waits for the relay connection before fetching members', () async {
      final session = _ConnectionAwareRelaySession();
      final container = ProviderContainer(
        retry: (_, _) => null,
        overrides: [relaySessionProvider.overrideWith(() => session)],
      );
      addTearDown(container.dispose);
      final subscription = container.listen(
        channelMembersProvider(_channelId),
        (_, _) {},
      );
      addTearDown(subscription.close);

      expect(
        await container.read(channelMembersProvider(_channelId).future),
        isEmpty,
      );
      expect(session.historyQueryCount, 0);

      session.connect();
      await container.pump();
      final members = await container.read(
        channelMembersProvider(_channelId).future,
      );

      expect(session.historyQueryCount, 1);
      expect(members, hasLength(1));
      expect(members.single.pubkey, _memberPubkey);
      expect(members.single.role, 'admin');
    });

    test(
      'keeps the provider member snapshot available during reconnect',
      () async {
        final session = _ConnectionAwareRelaySession();
        final container = ProviderContainer(
          retry: (_, _) => null,
          overrides: [relaySessionProvider.overrideWith(() => session)],
        );
        addTearDown(container.dispose);
        final subscription = container.listen(
          channelMembersProvider(_channelId),
          (_, _) {},
        );
        addTearDown(subscription.close);

        session.connect();
        await container.pump();
        final connectedMembers = await container.read(
          channelMembersProvider(_channelId).future,
        );
        expect(connectedMembers, hasLength(1));
        expect(session.historyQueryCount, 1);

        session.setStatus(SessionStatus.reconnecting);
        await container.pump();

        final reconnectingMembers = container
            .read(channelMembersProvider(_channelId))
            .asData
            ?.value;
        expect(reconnectingMembers, connectedMembers);
        expect(session.historyQueryCount, 1);
      },
    );

    test('keeps the member snapshot available during reconnect', () {
      final cachedMembers = [
        ChannelMember(
          pubkey: _memberPubkey,
          role: 'member',
          joinedAt: DateTime.fromMillisecondsSinceEpoch(1000),
        ),
      ];
      final refreshedMember = ChannelMember(
        pubkey: _memberPubkey,
        role: 'admin',
        joinedAt: DateTime.fromMillisecondsSinceEpoch(2000),
      );

      expect(
        channelMembersForAutocomplete(
          membersAsync: const AsyncData([]),
          sessionStatus: SessionStatus.connected,
          cachedMembers: cachedMembers,
        ),
        isEmpty,
      );
      expect(
        channelMembersForAutocomplete(
          membersAsync: const AsyncData([]),
          sessionStatus: SessionStatus.reconnecting,
          cachedMembers: cachedMembers,
        ),
        same(cachedMembers),
      );
      expect(
        channelMembersForAutocomplete(
          membersAsync: AsyncData([refreshedMember]),
          sessionStatus: SessionStatus.connected,
          cachedMembers: cachedMembers,
        ),
        [refreshedMember],
      );
    });
  });

  group('directory providers relay-config invalidation', () {
    NostrEvent profile(String pubkey, String name) => NostrEvent(
      id: '$pubkey-profile',
      pubkey: pubkey,
      createdAt: 1700000000,
      kind: 0,
      tags: const [],
      content: '{"display_name":"$name"}',
      sig: 'sig',
    );

    ProviderContainer buildContainer(_DirectoryFakeRelaySession session) {
      return ProviderContainer(
        retry: (_, _) => null,
        overrides: [
          relaySessionProvider.overrideWith(() => session),
          myPubkeyProvider.overrideWithValue('me'),
        ],
      );
    }

    test('browse directory refetches when the relay config changes', () async {
      final session = _DirectoryFakeRelaySession(
        profileEvents: [profile('alice', 'Alice')],
      );
      final container = buildContainer(session);
      addTearDown(container.dispose);

      // Keep the autoDispose provider alive across the config change, the
      // way an open New message sheet would.
      final subscription = container.listen(
        relayDirectoryUsersProvider,
        (_, _) {},
      );
      addTearDown(subscription.close);

      final firstUsers = await container.read(
        relayDirectoryUsersProvider.future,
      );
      expect(firstUsers.map((user) => user.label), ['Alice']);
      expect(session.directoryQueryCount, 1);

      // Simulate switching to a community that shares the same signing key:
      // session notifier instance and pubkey both survive; only the relay
      // config changes.
      session.profileEvents = [profile('bob', 'Bob')];
      container
          .read(relayConfigProvider.notifier)
          .update(baseUrl: 'http://other-community.example', nsec: null);
      await container.pump();

      final secondUsers = await container.read(
        relayDirectoryUsersProvider.future,
      );
      expect(secondUsers.map((user) => user.label), ['Bob']);
      expect(session.directoryQueryCount, 2);
    });

    test('search results refetch when the relay config changes', () async {
      final session = _DirectoryFakeRelaySession(
        profileEvents: [profile('alice', 'Alice')],
      );
      final container = buildContainer(session);
      addTearDown(container.dispose);

      final subscription = container.listen(
        relayDirectorySearchProvider('ali'),
        (_, _) {},
      );
      addTearDown(subscription.close);

      final firstResults = await container.read(
        relayDirectorySearchProvider('ali').future,
      );
      expect(firstResults.map((user) => user.label), ['Alice']);
      expect(session.searchQueryCount, 1);

      session.profileEvents = [profile('alina', 'Alina')];
      container
          .read(relayConfigProvider.notifier)
          .update(baseUrl: 'http://other-community.example', nsec: null);
      await container.pump();

      final secondResults = await container.read(
        relayDirectorySearchProvider('ali').future,
      );
      expect(secondResults.map((user) => user.label), ['Alina']);
      expect(session.searchQueryCount, 2);
    });

    test('cached search families are released once unlistened', () async {
      final session = _DirectoryFakeRelaySession(
        profileEvents: [profile('alice', 'Alice')],
      );
      final container = buildContainer(session);
      addTearDown(container.dispose);

      final subscription = container.listen(
        relayDirectorySearchProvider('ali'),
        (_, _) {},
      );
      await container.read(relayDirectorySearchProvider('ali').future);
      subscription.close();
      await container.pump();

      // A fresh read after disposal hits the relay again instead of reusing
      // a session-lifetime cache entry.
      await container.read(relayDirectorySearchProvider('ali').future);
      expect(session.searchQueryCount, 2);
    });
  });
}

const _channelId = '11111111-1111-4111-8111-111111111111';
const _memberPubkey =
    'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa';

class _RecordingPublishRelaySession extends RelaySessionNotifier {
  final publishedEvents = <NostrEvent>[];
  int historyQueryCount = 0;

  @override
  SessionState build() => const SessionState(status: SessionStatus.connected);

  @override
  Future<NostrEvent> publish(
    NostrEvent event, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    publishedEvents.add(event);
    return event;
  }

  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    historyQueryCount++;
    return const [];
  }
}

class _FixedRelayConfigNotifier extends RelayConfigNotifier {
  @override
  RelayConfig build() =>
      RelayConfig(baseUrl: 'https://first-community.example', nsec: null);

  @override
  void update({required String baseUrl, String? nsec}) {
    state = RelayConfig(baseUrl: baseUrl, nsec: nsec);
  }
}

class _ConnectionAwareRelaySession extends RelaySessionNotifier {
  int historyQueryCount = 0;

  @override
  SessionState build() =>
      const SessionState(status: SessionStatus.disconnected);

  void connect() {
    state = const SessionState(status: SessionStatus.connected);
  }

  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    historyQueryCount++;
    return [
      NostrEvent(
        id: 'members',
        pubkey: 'owner',
        createdAt: 1,
        kind: 39002,
        tags: const [
          ['d', _channelId],
          ['p', _memberPubkey, 'wss://relay.example', 'admin'],
        ],
        content: '',
        sig: 'sig',
      ),
    ];
  }

  void setStatus(SessionStatus status) {
    state = SessionState(status: status);
  }

  @override
  Future<void Function()> subscribe(
    NostrFilter filter,
    void Function(NostrEvent) onEvent, {
    void Function(String message)? onClosed,
  }) async => () {};
}

/// Fake [RelaySessionNotifier] that serves canned kind:0 profile events from
/// [queryRelay] and counts directory vs. search queries.
class _DirectoryFakeRelaySession extends RelaySessionNotifier {
  _DirectoryFakeRelaySession({required this.profileEvents});

  List<NostrEvent> profileEvents;
  int directoryQueryCount = 0;
  int searchQueryCount = 0;

  @override
  SessionState build() => const SessionState(status: SessionStatus.connected);

  @override
  Future<List<NostrEvent>> queryRelay(
    List<NostrFilter> filters, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    if (filters.any((filter) => filter.search != null)) {
      searchQueryCount++;
    } else {
      directoryQueryCount++;
    }
    return profileEvents;
  }

  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    return const [];
  }
}
