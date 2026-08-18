import 'package:flutter/material.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import 'channel.dart';
import 'channel_detail_page.dart';
import 'channels_provider.dart';

void openChannelLink({
  required BuildContext context,
  required WidgetRef ref,
  required String channelId,
  required String currentChannelId,
}) {
  if (channelId == currentChannelId) return;

  final channelsAsync = ref.read(channelsProvider);
  final channels = channelsAsync.hasValue ? channelsAsync.value : null;
  Channel? targetChannel;
  for (final channel in channels ?? const <Channel>[]) {
    if (channel.id == channelId) {
      targetChannel = channel;
      break;
    }
  }

  if (targetChannel == null) {
    ScaffoldMessenger.of(context).showSnackBar(
      const SnackBar(content: Text('Channel could not be opened')),
    );
    return;
  }

  Navigator.of(context).push(
    MaterialPageRoute<void>(
      builder: (_) => ChannelDetailPage(channel: targetChannel!),
    ),
  );
}
