import 'package:buzz/shared/widgets/feedback_snackbar.dart';

import 'package:flutter/material.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/me/default_agent.dart';
import '../../shared/me/me_sync.dart';
import '../../shared/relay/relay.dart';
import 'channel.dart';
import 'channel_detail_page.dart';
import 'channels_provider.dart';
import 'channel_management_provider.dart';

/// Opens the direct conversation with the Bestie [pubkey], reusing a loaded
/// one. [onResolving] reports while a new conversation is looked up.
Future<void> openBestieConversation(
  BuildContext context,
  WidgetRef ref,
  String pubkey, {
  ValueChanged<bool>? onResolving,
}) async {
  final scope = ref.read(meSyncProvider);
  try {
    final channelState = ref.read(channelsProvider);
    // AsyncValue may retain the previous community's list during reload.
    final channels = channelState.isLoading || channelState.hasError
        ? const <Channel>[]
        : channelState.value ?? const <Channel>[];
    final notifier = ref.read(channelsProvider.notifier);
    final cached = channels
        .where(
          (channel) => isActiveDirectConversation(
            channel,
            notifier.cachedMembersForChannel(channel.id),
            ref.read(myPubkeyProvider),
            pubkey,
          ),
        )
        .firstOrNull;
    onResolving?.call(cached == null);
    final channel =
        cached ??
        await ref.read(channelActionsProvider).openDm(pubkeys: [pubkey]);
    if (!context.mounted) return;
    onResolving?.call(false);
    // A community switch invalidates an in-flight destination.
    if (ref.read(meSyncProvider) != scope ||
        ref.read(defaultAgentProvider).isLoading ||
        ref.read(defaultAgentProvider).value != pubkey) {
      return;
    }
    await Navigator.of(context).push<void>(
      MaterialPageRoute(
        // Native glass and video layers must track the live route.
        allowSnapshotting: false,
        builder: (_) => ChannelDetailPage(channel: channel),
      ),
    );
  } catch (error) {
    if (context.mounted) {
      ScaffoldMessenger.of(context).showFeedbackSnackBar(
        SnackBar(
          content: Text('Could not open the agent conversation. $error'),
        ),
        kind: ToastKind.error,
      );
    }
  } finally {
    if (context.mounted) onResolving?.call(false);
  }
}
