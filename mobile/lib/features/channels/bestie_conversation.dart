import 'package:buzz/shared/widgets/feedback_snackbar.dart';

import 'package:flutter/material.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/widgets/adaptive_conversation_pane.dart';
import '../../shared/me/default_agent.dart';
import '../../shared/me/me_sync.dart';
import '../../shared/relay/relay.dart';
import 'channel.dart';
import 'channel_detail_page.dart';
import 'channels_provider.dart';
import 'channel_management_provider.dart';

/// The open whose conversation lookup is in flight.
Object? _lookup;

/// Opens the direct conversation with the Bestie [pubkey], reusing a loaded
/// one. [onResolving] reports while a new conversation is looked up. A call
/// while another open is still in flight does nothing, so the Home launcher
/// and the app shortcut can't stack two copies of the conversation.
Future<void> openBestieConversation(
  BuildContext context,
  WidgetRef ref,
  String pubkey, {
  ValueChanged<bool>? onResolving,
}) async {
  if (_lookup != null) return;
  final lookup = _lookup = Object();
  void release() {
    if (_lookup == lookup) _lookup = null;
  }

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
    // A community switch or a newer screen invalidates an in-flight
    // destination.
    if (ref.read(meSyncProvider) != scope ||
        ModalRoute.of(context)?.isCurrent == false ||
        ref.read(defaultAgentProvider).isLoading ||
        ref.read(defaultAgentProvider).value != pubkey) {
      return;
    }
    // Only the lookup is guarded; an open conversation can be replaced.
    release();
    final pane = ConversationPaneScope.maybeOf(context);
    if (pane != null) {
      pane.open(channel.id, (_) => ChannelDetailPage(channel: channel));
    } else {
      await Navigator.of(context).push<void>(
        MaterialPageRoute(
          // Native glass and video layers must track the live route.
          allowSnapshotting: false,
          builder: (_) => ChannelDetailPage(channel: channel),
        ),
      );
    }
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
    release();
    if (context.mounted) onResolving?.call(false);
  }
}
