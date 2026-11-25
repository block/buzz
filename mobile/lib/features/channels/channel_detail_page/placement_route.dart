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
    final details = ref.watch(channelDetailsProvider(channel.id)).value;
    final current = details == null ? channel : channel.mergeDetails(details);
    final placed =
        current.isPersonalChat(
          ref.watch(myPubkeyProvider),
          groups?.channels ?? const [],
        ) ||
        personalChat ||
        placement == ChannelRoutePlacement.solo;
    final lookup = ref.watch(meSearchPlacementProvider(channel.id));
    if (!placed &&
        placement != ChannelRoutePlacement.ordinary &&
        groups == null &&
        lookup.isLoading) {
      return FrostedScaffold(
        appBar: FrostedAppBar(title: Text(channel.name)),
        body: const Center(child: BuzzLoadingIndicator()),
      );
    }
    return CoveredRouteKeyboardInsets(
      child: _ResolvedChannelDetailPage(
        key: ValueKey((channel.id, placed)),
        channel: current,
        personalChat: placed,
        welcomeAgent: current.isDm
            ? ref.watch(bestieWelcomeProvider(channel.id))
            : null,
        placementKnown:
            placed ||
            groups != null ||
            placement != ChannelRoutePlacement.unresolved,
        initialAssistant: initialAssistant,
        initialMessageId: initialMessageId,
        initialThreadRootId: initialThreadRootId,
        initialThreadRouteBehavior: initialThreadRouteBehavior,
      ),
    );
  }
}

VoidCallback _useCloseChatAfterSettings(BuildContext context) {
  final route = ModalRoute.of(context);
  final paneOwner = ConversationPaneScope.isBaseRouteOf(context);
  final closeOnReveal = useState(false);

  useEffect(() {
    if (closeOnReveal.value && route?.isCurrent == true) {
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (route?.isCurrent == true) route!.navigator?.pop();
      });
    }
    return null;
  }, [route, route?.isCurrent, closeOnReveal.value]);

  return () {
    if (route == null || !route.isActive) return;
    if (route.isCurrent) {
      route.navigator?.pop();
    } else if (paneOwner || route.isFirst) {
      // Keep the navigator and its visible page intact until the base chat
      // is revealed again.
      if (context.mounted) closeOnReveal.value = true;
    } else {
      route.navigator?.removeRoute(route);
    }
  };
}
