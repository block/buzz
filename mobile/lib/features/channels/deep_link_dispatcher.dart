import 'package:flutter/material.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/deeplink/deep_link.dart';
import '../../shared/deeplink/pending_deep_link_provider.dart';
import 'channel.dart';
import 'channel_detail_page.dart';
import 'channels_provider.dart';

/// Routes pending `buzz://message` deep links into the channel view.
///
/// Wraps the authenticated home subtree. Whenever a parsed link is parked in
/// [pendingDeepLinkProvider] and the channel list is available, this pushes
/// the target [ChannelDetailPage] on the enclosing [Navigator]. Links are
/// held (not dropped) while channels are still loading, so cold-start links
/// dispatch as soon as the first channel fetch completes.
typedef DeepLinkDestinationBuilder =
    Widget Function(Channel channel, MessageDeepLink link);

class DeepLinkDispatcher extends ConsumerStatefulWidget {
  final Widget child;
  final DeepLinkDestinationBuilder? destinationBuilder;

  const DeepLinkDispatcher({
    super.key,
    required this.child,
    this.destinationBuilder,
  });

  @override
  ConsumerState<DeepLinkDispatcher> createState() => _DeepLinkDispatcherState();
}

class _DeepLinkDispatcherState extends ConsumerState<DeepLinkDispatcher> {
  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (!mounted) return;
      _maybeDispatch(ref.read(pendingDeepLinkProvider));
    });
  }

  @override
  Widget build(BuildContext context) {
    // Re-evaluate dispatch when either a new link arrives or channels load.
    ref.listen<MessageDeepLink?>(pendingDeepLinkProvider, (_, link) {
      _maybeDispatch(link);
    });
    ref.listen<AsyncValue<List<Channel>>>(channelsProvider, (_, _) {
      _maybeDispatch(ref.read(pendingDeepLinkProvider));
    });

    return widget.child;
  }

  void _maybeDispatch(MessageDeepLink? link) {
    if (link == null) return;

    final channels = ref.read(channelsProvider).asData?.value;
    // Channels not loaded yet — keep the link parked; the channelsProvider
    // listener re-attempts once data arrives.
    if (channels == null) return;

    ref.read(pendingDeepLinkProvider.notifier).consume();

    final channel = channels
        .where((c) => c.id == link.channelId)
        .cast<Channel?>()
        .firstOrNull;
    if (channel == null) {
      debugPrint(
        'deep-link: channel ${link.channelId} not found in workspace; '
        'dropping link',
      );
      ScaffoldMessenger.maybeOf(context)?.showSnackBar(
        const SnackBar(content: Text('Channel not found in this workspace')),
      );
      return;
    }
    if (!context.mounted) return;

    Navigator.of(context).push(
      MaterialPageRoute<void>(
        builder: (_) =>
            widget.destinationBuilder?.call(channel, link) ??
            ChannelDetailPage(
              channel: channel,
              initialMessageId: link.messageId,
              initialThreadRootId: link.threadRootId,
            ),
      ),
    );
  }
}
