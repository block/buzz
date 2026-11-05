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

/// Taps still resolving, so a repeated tap during a slow lookup doesn't open
/// the channel twice. Each is keyed by the page and connection it was made
/// on, so a lookup that will be discarded blocks no new tap: the relay config
/// is replaced on every community or identity switch, even back to an earlier
/// one, and a closed page's lookup is dropped.
final _resolving = <(Object?, RelayConfig, String)>{};

/// Resolves [channelId] for a tap that opens it, or shows why it can't be
/// opened and returns null.
///
/// The loaded list holds member channels and, once loaded, the open
/// directory. An open channel outside both is read from its own metadata, as
/// `buzz://` links do, and remembered so the opened page can find it.
/// Returns null without feedback for a repeated tap while the first one is
/// resolving, or when the community or identity changed or the user moved to
/// another page during the lookup.
Future<Channel?> resolveChannelToOpen(
  BuildContext context,
  WidgetRef ref,
  String channelId,
) async {
  final key = (
    ModalRoute.of(context),
    ref.read(relayConfigProvider),
    channelId,
  );
  if (!_resolving.add(key)) return null;
  try {
    return await _resolve(context, ref, channelId);
  } finally {
    _resolving.remove(key);
  }
}

Future<Channel?> _resolve(
  BuildContext context,
  WidgetRef ref,
  String channelId,
) async {
  final notifier = ref.read(channelsProvider.notifier);
  final messenger = ScaffoldMessenger.of(context);
  final connection = ref.read(relayConfigProvider);
  Channel? channel;
  Object? failure;
  try {
    // A tap can arrive before the first list load finishes. A failed load
    // still leaves the channel's own metadata, so the tap is a real retry,
    // but without the list a channel the user belongs to may be unreadable.
    if (!ref.read(channelsProvider).hasValue) {
      try {
        await ref.read(channelsProvider.future);
      } catch (error) {
        failure = error;
      }
    }
    channel =
        notifier.channelById(channelId) ??
        await notifier.fetchOpenChannel(channelId);
  } catch (error) {
    failure = error;
  }
  // The tap belongs to the connection and page it was made on: once the user
  // has switched community or moved to another page, it opens nothing.
  if (!context.mounted ||
      !identical(ref.read(relayConfigProvider), connection) ||
      !(ModalRoute.of(context)?.isCurrent ?? true)) {
    return null;
  }
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
