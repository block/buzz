import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:buzz/features/channels/channel_management_provider.dart';
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
