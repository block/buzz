part of '../channel_detail_page.dart';

/// Explicit destination identity; generic entries allow ordinary sending.
enum ChannelRoutePlacement { lookup, ordinary, solo, unresolved }

/// Resolves Solo placement at the destination, including Activity/deep links.
class ChannelDetailPage extends ConsumerWidget {
  final Channel channel;
  final bool personalChat;

  /// Caller identity; known Solo organization always takes precedence.
  final ChannelRoutePlacement placement;
  final String? initialAssistant;
  final String? initialMessageId;
  final String? initialThreadRootId;
  final InitialThreadRouteBehavior initialThreadRouteBehavior;

  const ChannelDetailPage({
    super.key,
    required this.channel,
    this.personalChat = false,
    this.placement = ChannelRoutePlacement.lookup,
    this.initialAssistant,
    this.initialMessageId,
    this.initialThreadRootId,
    this.initialThreadRouteBehavior = InitialThreadRouteBehavior.push,
  });

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final groups = ref.watch(meGroupsProvider).value;
    final placed =
        personalChat ||
        placement == ChannelRoutePlacement.solo ||
        (groups?.channels.contains(channel.id) ?? false);
    final lookup = ref.watch(meSearchPlacementProvider(channel.id));
    if (!placed &&
        placement != ChannelRoutePlacement.ordinary &&
        groups == null &&
        lookup.isLoading) {
      return Scaffold(
        appBar: AppBar(title: Text(channel.name)),
        body: const Center(child: BuzzLoadingIndicator()),
      );
    }
    return _ResolvedChannelDetailPage(
      key: ValueKey((channel.id, placed)),
      channel: channel,
      personalChat: placed,
      placementKnown:
          placed ||
          groups != null ||
          placement != ChannelRoutePlacement.unresolved,
      initialAssistant: initialAssistant,
      initialMessageId: initialMessageId,
      initialThreadRootId: initialThreadRootId,
      initialThreadRouteBehavior: initialThreadRouteBehavior,
    );
  }
}
