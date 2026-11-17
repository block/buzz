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
      final subscription = container.listen(
        channelPermissionsProvider('room'),
        (_, _) {},
      );
      addTearDown(subscription.close);
      expect(
        container
            .read(channelPermissionsProvider('room'))
            .allows(ChannelAction.post),
        isTrue,
      );
      role = 'guest';
      container.invalidate(latestChannelMembersProvider('room'));
      expect(
        container
            .read(channelPermissionsProvider('room'))
            .allows(ChannelAction.post),
        isFalse,
      );
      expect(
        container
            .read(channelPermissionsProvider('room'))
            .allows(ChannelAction.reply),
        isTrue,
      );
      room = channel();
      container.invalidate(channelByIdProvider('room'));
      expect(
        container
            .read(channelPermissionsProvider('room'))
            .allows(ChannelAction.reply),
        isFalse,
      );
      expect(
        container
            .read(channelPermissionsProvider('room'))
            .allows(ChannelAction.react),
        isFalse,
      );
    },
  );

  test('viewer grants are independent and never grant root publishing', () {
    for (final replies in [false, true]) {
      for (final reactions in [false, true]) {
        final room = channel(replies: replies, reactions: reactions);
        final guest = ChannelPermissions(room, 'guest');
        expect(guest.allows(ChannelAction.post), isFalse);
        expect(guest.allows(ChannelAction.reply), replies);
        expect(guest.allows(ChannelAction.react), reactions);
        expect(guest.allows(ChannelAction.removeReaction), isTrue);
        for (final role in ['member', 'admin', 'owner', 'bot']) {
          for (final action in ChannelAction.values) {
            expect(ChannelPermissions(room, role).allows(action), isTrue);
          }
        }
        for (final action in ChannelAction.values) {
          expect(ChannelPermissions(room, null).allows(action), isFalse);
          expect(
            ChannelPermissions(
              channel(replies: true, reactions: true, joined: false),
              'guest',
            ).allows(action),
            isFalse,
          );
          expect(
            ChannelPermissions(
              channel(replies: true, reactions: true, archived: true),
              'guest',
            ).allows(action),
            isFalse,
          );
        }
        final restored = Channel.fromJson(room.toJson());
        expect(restored.guestReplies, replies);
        expect(restored.copyWith(name: 'Updated').guestReactions, reactions);
        expect(
          restored.mergeDetails(ChannelDetails.fromChannel(room)).guestReplies,
          replies,
        );
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
