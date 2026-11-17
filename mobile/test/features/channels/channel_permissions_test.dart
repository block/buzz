import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:buzz/features/channels/channel_management_provider.dart';
import 'package:buzz/features/channels/channels_provider.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channel_permissions.dart';
import 'package:flutter_test/flutter_test.dart';

Channel channel({
  bool replies = false,
  bool reactions = false,
  bool joined = true,
  bool archived = false,
}) => Channel(
  id: 'room',
  name: 'Room',
  channelType: 'stream',
  visibility: 'open',
  description: '',
  createdBy: 'owner',
  createdAt: DateTime.utc(2026),
  memberCount: 2,
  isMember: joined,
  archivedAt: archived ? DateTime.utc(2026) : null,
  guestReplies: replies,
  guestReactions: reactions,
);

void main() {
  test(
    'live roster changes and grant revocation update effective permissions',
    () {
      var role = 'member';
      var room = channel(replies: true, reactions: true);
      final container = ProviderContainer(
        overrides: [
          myPubkeyProvider.overrideWithValue('viewer'),
          channelByIdProvider('room').overrideWith((ref) => room),
          latestChannelMembersProvider('room').overrideWith(
            (ref) => [
              ChannelMember(
                pubkey: 'viewer',
                role: role,
                joinedAt: DateTime.utc(2026),
              ),
            ],
          ),
        ],
      );
      addTearDown(container.dispose);
      final provider = channelPermissionsProvider('room');
      final subscription = container.listen(provider, (_, _) {});
      addTearDown(subscription.close);
      bool allows(ChannelAction action) =>
          container.read(provider).allows(action);

      expect(allows(ChannelAction.post), isTrue);
      role = 'guest';
      container.invalidate(latestChannelMembersProvider('room'));
      expect(allows(ChannelAction.post), isFalse);
      expect(allows(ChannelAction.reply), isTrue);
      expect(allows(ChannelAction.react), isTrue);

      room = channel();
      container.invalidate(channelByIdProvider('room'));
      expect(allows(ChannelAction.reply), isFalse);
      expect(allows(ChannelAction.react), isFalse);
    },
  );

  test('viewer grants are independent and never grant root publishing', () {
    for (final replies in [false, true]) {
      for (final reactions in [false, true]) {
        final guest = ChannelPermissions(
          channel(replies: replies, reactions: reactions),
          'guest',
        );
        expect(guest.allows(ChannelAction.post), isFalse);
        expect(guest.allows(ChannelAction.reply), replies);
        expect(guest.allows(ChannelAction.react), reactions);
        expect(guest.allows(ChannelAction.removeReaction), isTrue);
      }
    }

    final room = channel();
    for (final role in ['member', 'admin', 'owner', 'bot']) {
      expect(
        ChannelAction.values.every(ChannelPermissions(room, role).allows),
        isTrue,
      );
    }
    for (final permissions in [
      ChannelPermissions(room, null),
      ChannelPermissions(
        channel(replies: true, reactions: true, joined: false),
        'guest',
      ),
      ChannelPermissions(
        channel(replies: true, reactions: true, archived: true),
        'guest',
      ),
    ]) {
      expect(ChannelAction.values.any(permissions.allows), isFalse);
    }
  });

  test('channel snapshots and updates preserve independent grants', () {
    for (final replies in [false, true]) {
      final original = channel(replies: replies, reactions: !replies);
      final restored = Channel.fromJson(original.toJson());
      for (final room in [
        restored,
        restored.copyWith(name: 'Updated'),
        channel().mergeDetails(ChannelDetails.fromChannel(original)),
      ]) {
        expect(room.guestReplies, replies);
        expect(room.guestReactions, !replies);
      }
    }
  });

  test('only exact, singular metadata flags grant contributions', () {
    for (final name in ['guest_replies', 'guest_reactions']) {
      for (final tags in <List<List<String>>>[
        [],
        [
          [name],
        ],
        [
          [name, 'false'],
        ],
        [
          [name, 'yes'],
        ],
        [
          [name, 'true', 'extra'],
        ],
        [
          [name, 'true'],
          [name, 'true'],
        ],
        [
          [name, 'true'],
        ],
      ]) {
        final metadata = NostrEvent(
          id: 'metadata',
          pubkey: 'relay',
          createdAt: 1,
          kind: 39000,
          tags: tags,
          content: '',
          sig: '',
        );
        final data = ChannelData.fromEvent(metadata);
        expect(
          name == 'guest_replies' ? data.guestReplies : data.guestReactions,
          tags.length == 1 &&
              tags.single.length == 2 &&
              tags.single[1] == 'true',
        );
      }
    }
  });
}
