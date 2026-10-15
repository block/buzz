part of '../channel_detail_page.dart';

/// Resolves Solo placement at the destination, including Activity/deep links.
class ChannelDetailPage extends ConsumerWidget {
  final Channel channel;
  final bool personalChat;
  final String? initialAssistant;
  final String? initialMessageId;
  final String? initialThreadRootId;
  final InitialThreadRouteBehavior initialThreadRouteBehavior;

  const ChannelDetailPage({
    super.key,
    required this.channel,
    this.personalChat = false,
    this.initialAssistant,
    this.initialMessageId,
    this.initialThreadRootId,
    this.initialThreadRouteBehavior = InitialThreadRouteBehavior.push,
  });

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final groups = ref.watch(meGroupsProvider).value;
    final fallback = ref.watch(meSearchPlacementProvider(channel.id));
    // Solo chats are private streams. Public channels, forums and DMs do not
    // depend on the optional Solo record to use their ordinary send path.
    final ordinary =
        !personalChat &&
        (channel.visibility == 'open' || channel.isDm || channel.isForum);
    final placed = groups?.channels.contains(channel.id) ?? personalChat;
    // Bounded waiting keeps ordinary search results readable during an outage.
    // Unknown placement may be read, but cannot send via an unverified path.
    if (!ordinary && !placed && groups == null && fallback.isLoading) {
      return Scaffold(
        appBar: AppBar(title: Text(channel.name)),
        body: const Center(child: BuzzLoadingIndicator()),
      );
    }
    return _ResolvedChannelDetailPage(
      key: ValueKey((channel.id, placed)),
      channel: channel,
      personalChat: placed,
      placementKnown: ordinary || groups != null || personalChat,
      initialAssistant: initialAssistant,
      initialMessageId: initialMessageId,
      initialThreadRootId: initialThreadRootId,
      initialThreadRouteBehavior: initialThreadRouteBehavior,
    );
  }
}
