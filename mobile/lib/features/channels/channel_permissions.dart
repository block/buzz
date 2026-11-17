import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/relay/relay.dart';
import 'channel.dart';
import 'channel_management_provider.dart';
import 'channels_provider.dart';

enum ChannelAction { post, reply, react, removeReaction }

/// Admission defaults never describe an existing participant's authority.
class ChannelPermissions {
  final Channel? channel;
  final String? role;

  const ChannelPermissions(this.channel, this.role);

  bool allows(ChannelAction action) {
    final channel = this.channel;
    if (channel == null || !channel.isMember || channel.isArchived) {
      return false;
    }
    if (channel.isDm ||
        const ['owner', 'admin', 'member', 'bot'].contains(role)) {
      return true;
    }
    if (role != 'guest' || !(channel.isStream || channel.isForum)) return false;
    return switch (action) {
      ChannelAction.post => false,
      ChannelAction.reply => channel.guestReplies,
      ChannelAction.react => channel.guestReactions,
      ChannelAction.removeReaction => true,
    };
  }
}

/// Reuses the page's roster cache and its existing live invalidation path.
final channelPermissionsProvider = Provider.autoDispose
    .family<ChannelPermissions, String>((ref, channelId) {
      final channel = ref.watch(channelByIdProvider(channelId));
      final viewer = ref.watch(myPubkeyProvider)?.toLowerCase();
      final members = ref.watch(latestChannelMembersProvider(channelId));
      final role = members
          ?.where((member) => member.pubkey.toLowerCase() == viewer)
          .firstOrNull
          ?.role;
      return ChannelPermissions(channel, role);
    });
