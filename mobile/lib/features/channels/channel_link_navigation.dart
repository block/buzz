import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/widgets/feedback_snackbar.dart';
import 'package:flutter/material.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import 'channel.dart';
import 'channel_detail_page.dart';
import 'channels_provider.dart';

Future<void> openChannelLink({
  required BuildContext context,
  required WidgetRef ref,
  required String channelId,
  required String currentChannelId,
}) async {
  if (channelId == currentChannelId) return;

  final targetChannel = await resolveChannelToOpen(context, ref, channelId);
  if (targetChannel == null || !context.mounted) return;

  Navigator.of(context).push(
    MaterialPageRoute<void>(
      builder: (_) => ChannelDetailPage(channel: targetChannel),
    ),
  );
}

/// Channel ids whose tap is still resolving, so a repeated tap during a slow
/// lookup doesn't open the channel twice.
final _resolving = <String>{};

/// Resolves [channelId] for a tap that opens it, or shows why it can't be
/// opened and returns null.
///
/// The loaded list holds member channels and, once loaded, the open
/// directory. An open channel outside both is read from its own metadata, as
/// `buzz://` links do, and remembered so the opened page can find it.
/// Returns null without feedback for a repeated tap while the first one is
/// resolving, or when the community or identity changed during the lookup.
Future<Channel?> resolveChannelToOpen(
  BuildContext context,
  WidgetRef ref,
  String channelId,
) async {
  if (!_resolving.add(channelId)) return null;
  try {
    return await _resolve(context, ref, channelId);
  } finally {
    _resolving.remove(channelId);
  }
}

Future<Channel?> _resolve(
  BuildContext context,
  WidgetRef ref,
  String channelId,
) async {
  final notifier = ref.read(channelsProvider.notifier);
  final messenger = ScaffoldMessenger.of(context);
  String scope() => channelDirectoryScope(
    ref.read(relayConfigProvider).baseUrl,
    ref.read(myPubkeyProvider),
  );
  final startScope = scope();
  Channel? channel;
  Object? failure;
  try {
    // A tap can arrive before the first list load finishes.
    if (!ref.read(channelsProvider).hasValue) {
      await ref.read(channelsProvider.future);
    }
    channel =
        notifier.channelById(channelId) ??
        await notifier.fetchOpenChannel(channelId);
  } catch (error) {
    failure = error;
  }
  // The tap belongs to the community it was made in.
  if (!context.mounted || scope() != startScope) return null;
  if (channel != null) {
    notifier.rememberOpened(channel);
    return channel;
  }

  if (failure != null) {
    debugPrint('channel $channelId lookup failed: $failure');
  }
  messenger.showFeedbackSnackBar(
    SnackBar(
      content: Text(
        failure != null
            ? 'Could not open the channel'
            : 'Channel not found here',
      ),
    ),
    kind: ToastKind.error,
  );
  return null;
}
