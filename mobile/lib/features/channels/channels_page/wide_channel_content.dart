part of '../channels_page.dart';

/// Nested tablet channel detail surface resolved from a selected channel ID.
class WideChannelContent extends HookConsumerWidget {
  final String channelId;
  final VoidCallback onChannelLeft;

  const WideChannelContent({
    super.key,
    required this.channelId,
    required this.onChannelLeft,
  });

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final channels = ref.watch(channelsProvider).asData?.value;
    final channel = channels?.cast<Channel?>().firstWhere(
      (channel) => channel?.id == channelId,
      orElse: () => null,
    );
    if (channel == null) return const SizedBox.shrink();
    final navigatorKey = useMemoized(GlobalKey<NavigatorState>.new, [
      channelId,
    ]);
    return NavigatorPopHandler(
      key: ValueKey('wide-channel-detail-$channelId'),
      onPopWithResult: (_) => navigatorKey.currentState?.maybePop(),
      child: Navigator(
        key: navigatorKey,
        onGenerateRoute: (_) => MaterialPageRoute<void>(
          builder: (_) =>
              ChannelDetailPage(channel: channel, onChannelLeft: onChannelLeft),
        ),
      ),
    );
  }
}
