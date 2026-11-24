import 'package:flutter/widgets.dart';

import '../../shared/theme/buzz_icons.dart';
import 'channel.dart';

/// The glyph the Messages list leads a Solo chat's row with.
const soloChatListIcon = BuzzIcons.messageCircle;

/// The glyph the Messages list leads [channel]'s row with, so that anything
/// else naming a conversation by its kind shows the same one.
///
/// [legacySoloChannels] are the old mobile-created Solo chats, which carry no
/// Solo label. A DM has no glyph: its row leads with its people's avatars.
IconData? conversationListIcon(
  Channel channel, {
  Iterable<String> legacySoloChannels = const [],
}) {
  if (channel.isSoloChat(legacySoloChannels)) return soloChatListIcon;
  if (channel.isDm) return null;
  return channel.isPrivate ? BuzzIcons.lock : BuzzIcons.hash;
}
