import 'package:flutter_test/flutter_test.dart';
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
}
