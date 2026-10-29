import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/agents/remote_agents.dart';
import '../../shared/community/community_provider.dart';
import '../../shared/hosted/hosted_flow.dart';
import '../../shared/hosted/hosted_flow_provider.dart';
import '../../shared/identity_names/identity_names_provider.dart';
import '../../shared/mentions/agent_identity_provider.dart';
import '../../shared/profile/user_cache_provider.dart';
import '../../shared/relay/relay.dart';
import '../../shared/theme/theme.dart';
import '../../shared/widgets/modal_presentation.dart';
import '../../shared/widgets/agent_navigation_icon.dart';
import '../channels/channel.dart';
import '../channels/channel_detail_page.dart';
import '../profile/user_profile_sheet.dart';
import 'create_agent_page.dart';
import 'agent_card.dart';
import 'remote_agent_model_control.dart';
import 'owned_agent_directory.dart';
import '../../shared/widgets/frosted_app_bar.dart';
import '../../shared/widgets/frosted_scaffold.dart';
import '../../shared/theme/buzz_icons.dart';
import 'agent_card_grid.dart';
import 'delete_agent_action.dart';
import 'bestie_agent_action.dart';

/// Opens the active community's full-page Agents navigation stack.
Future<void> showAgents(BuildContext context) async {
  final container = ProviderScope.containerOf(context);
  final controller = container.read(remoteAgentControllerProvider);
  unawaited(controller.run(controller.refresh));
  final channel = await Navigator.of(context).push<Channel>(
    MaterialPageRoute(
      builder: (_) => _AgentsNavigation(controller: controller),
    ),
  );
  if (channel != null && context.mounted) {
    await Navigator.of(context).push<void>(
      MaterialPageRoute(builder: (_) => ChannelDetailPage(channel: channel)),
    );
  }
}

class _AgentsNavigation extends HookConsumerWidget {
  const _AgentsNavigation({required this.controller});

  final RemoteAgentController controller;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final communityId = ref.watch(activeCommunityProvider).asData?.value?.id;
    final navigator = useMemoized(() => GlobalKey<NavigatorState>(), [
      communityId,
    ]);
    return NavigatorPopHandler<Channel>(
      onPopWithResult: (result) {
        if (navigator.currentState?.canPop() ?? false) {
          navigator.currentState!.pop(result);
        }
      },
      child: Navigator(
        key: navigator,
        onGenerateRoute: (_) => MaterialPageRoute<void>(
          builder: (listContext) => Material(
            color: context.colors.surface,
            child: AgentsPage(
              controller: controller,
              onClose: () => Navigator.of(context).pop(),
              onSelect: (pubkey) async {
                final channel = await showBuzzModalBottomSheet<Channel>(
                  context: listContext,
                  isScrollControlled: true,
                  showDragHandle: false,
                  showCloseButton: false,
                  padBottom: false,
                  builder: (_) => UserProfileSheet(
                    pubkey: pubkey,
                    agentPresentation: true,
                    agentSubtitleIcon:
                        controller.agents.any(
                          (agent) => agent['agent_pubkey'] == pubkey,
                        )
                        ? const AgentNavigationIcon(size: 16)
                        : null,
                    agentSubtitle:
                        controller.agents.any(
                          (agent) => agent['agent_pubkey'] == pubkey,
                        )
                        ? 'Builderbot agent'
                        : 'Manage on desktop',
                    primaryActionsSpacing: Grid.md,
                    contextualActionsSpacing: Grid.xxs,
                    contextualActions: (_) => BestieAgentAction(pubkey: pubkey),
                    headerLeading: DeleteAgentAction(
                      controller: controller,
                      pubkey: pubkey,
                    ),
                  ),
                );
                if (channel != null && context.mounted) {
                  Navigator.of(context).pop(channel);
                }
              },
            ),
          ),
        ),
      ),
    );
  }
}

/// The current user's verified agents in the active community.
class AgentsPage extends HookConsumerWidget {
  const AgentsPage({
    required this.controller,
    required this.onClose,
    required this.onSelect,
    super.key,
  });
  final VoidCallback onClose;
  final ValueChanged<String> onSelect;
  final RemoteAgentController controller;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    useListenable(controller);
    useListenable(controller.account.subjectChanges);
    final flow = ref.watch(hostedFlowProvider);
    useListenable(flow);
    final signingIn =
        flow.reconnecting &&
        (flow.stage == HostedFlowStage.signIn ||
            flow.stage == HostedFlowStage.working);
    // Account sign-in belongs to the shared hosted flow; reload once it lands.
    useEffect(() {
      if (flow.reconnecting && flow.stage == HostedFlowStage.complete) {
        unawaited(controller.run(controller.refresh));
      }
      return null;
    }, [flow.stage]);
    final community = ref.watch(activeCommunityProvider).asData?.value;
    final directory = ref.watch(agentDirectoryProvider);
    final ownedDirectory = ref.watch(ownedAgentDirectoryProvider);
    final candidates = {
      if (controller.owner == community?.pubkey)
        for (final agent in controller.agents) agent['agent_pubkey'] as String,
      for (final entry in directory.asData?.value ?? <AgentDirectoryEntry>[])
        entry.pubkey,
    };
    final candidateKey = (candidates.toList()..sort()).join(',');
    final attempt = useState(0);
    final registryFuture = useMemoized(() async {
      final fetch = ref.read(relaySessionProvider.notifier).fetchHistory;
      return ownedCommunityAgents(candidates, community?.pubkey, fetch);
    }, [community?.relayUrl, community?.pubkey, candidateKey, attempt.value]);
    final registered = useFuture(registryFuture, preserveState: false);
    final scope = '${community?.relayUrl}|${community?.pubkey}|$candidateKey';
    final lastOwned = useRef<(String, Set<String>)?>(null);
    if (registered.data case final owned?) lastOwned.value = (scope, owned);
    final owned =
        registered.data ??
        (lastOwned.value?.$1 == scope ? lastOwned.value?.$2 : null);
    final keys = {
      ...?owned,
      for (final entry
          in ownedDirectory.asData?.value ?? <AgentDirectoryEntry>[])
        entry.pubkey,
    }.where((key) => !controller.deletedAgents.contains(key)).toList();
    final names = watchIdentityNames(
      ref,
      keys,
      agentPubkeys: keys.toSet(),
      fallbackNames: {
        for (final entry
            in ownedDirectory.asData?.value ?? <AgentDirectoryEntry>[])
          if (entry.displayName != null) entry.pubkey: entry.displayName!,
        for (final agent in controller.agents)
          agent['agent_pubkey'] as String: agent['agent_name'] as String,
      },
    );
    keys.sort((a, b) => names.labelFor(a).compareTo(names.labelFor(b)));
    final profiles = ref.watch(userCacheProvider);
    final creating = useState(false);
    final failed =
        directory.hasError ||
        registered.hasError ||
        ownedDirectory.hasError ||
        controller.listingFailed;
    final loading =
        !creating.value &&
        (controller.busy ||
            directory.isLoading ||
            ownedDirectory.isLoading ||
            registered.connectionState != ConnectionState.done);

    Future<void> refresh() async {
      await controller.run(controller.refresh);
      if (context.mounted) {
        attempt.value++;
        ref.invalidate(agentDirectoryProvider);
        ref.invalidate(ownedAgentDirectoryProvider);
      }
    }

    final VoidCallback? addAgent =
        community == null || controller.busy || creating.value || signingIn
        ? null
        : () async {
            if (controller.account.account == null) {
              await flow.reconnectAccount();
              return;
            }
            creating.value = true;
            try {
              await Navigator.of(context).push<bool>(
                MaterialPageRoute(
                  builder: (_) => CreateAgentPage(
                    controller: controller,
                    community: community,
                    onCreated: () => ref.invalidate(agentDirectoryProvider),
                  ),
                ),
              );
            } finally {
              if (context.mounted) creating.value = false;
            }
          };

    final headerStyle = context.textTheme.titleMedium?.copyWith(
      fontSize: 22,
      fontWeight: FontWeight.w600,
      color: navigationPrimaryForeground(context),
    );
    final topHeight = frostedAppBarHeight(
      context,
      nativeLargeTitle: true,
      titleStyle: headerStyle,
      bottomHeight: Grid.xxs,
    );
    return FrostedScaffold(
      backgroundColor: context.colors.surface,
      appBar: FrostedAppBar(
        nativeActions: [
          if (supportsRemoteAgents(community))
            IosNavigationAction(
              label: 'Add agent',
              symbol: 'plus',
              onPressed: addAgent,
            ),
        ],
        nativeTitle: 'Agents',
        nativeLargeTitle: true,
        automaticallyImplyLeading: false,
        nativeLeading: IosNavigationAction(
          label: 'Close agents',
          symbol: 'xmark',
          onPressed: onClose,
        ),
        leading: IconButton(
          tooltip: 'Close agents',
          icon: const Icon(BuzzIcons.x),
          onPressed: onClose,
        ),
        horizontalInset: Grid.gutter,
        showBottomDivider: true,
        bottomDividerOpacity: 0.07,
        title: Text('Agents', style: headerStyle),
        titleStyle: headerStyle,
        bottomHeight: Grid.xxs,
        bottom: const SizedBox.expand(),
        actions: [
          if (supportsRemoteAgents(community))
            IconButton(
              tooltip: 'Add agent',
              onPressed: addAgent,
              icon: const Icon(Icons.add),
            ),
        ],
      ),
      body: SafeArea(
        top: false,
        child: Material(
          color: Colors.transparent,
          child: RefreshIndicator(
            onRefresh: refresh,
            child: CustomScrollView(
              physics: const AlwaysScrollableScrollPhysics(),
              slivers: [
                SliverToBoxAdapter(child: SizedBox(height: topHeight)),
                SliverToBoxAdapter(
                  child: Column(
                    children: [
                      if (failed && keys.isEmpty)
                        ListTile(
                          title: const Text('Could not load agents'),
                          trailing: TextButton(
                            onPressed: refresh,
                            child: const Text('Retry'),
                          ),
                        ),
                      if (!loading && !failed && keys.isEmpty)
                        const Padding(
                          padding: EdgeInsets.all(Grid.sm),
                          child: Text(
                            'You haven’t added any agents yet.',
                            textAlign: TextAlign.center,
                          ),
                        ),
                    ],
                  ),
                ),
                AgentCardGrid(
                  itemCount: keys.length,
                  itemBuilder: (context, index) {
                    final key = keys[index];
                    final remote = controller.agents
                        .where((row) => row['agent_pubkey'] == key)
                        .firstOrNull;
                    final editable =
                        community != null &&
                        supportsRemoteAgents(community) &&
                        controller.owner == community.pubkey &&
                        owned?.contains(key) == true &&
                        controller.activeAgents.contains(key) &&
                        remote != null;
                    return AgentCard(
                      key: ValueKey(key),
                      name: names.labelFor(key),
                      imageUrl:
                          profiles[key]?.avatarUrl ??
                          ownedDirectory.asData?.value
                              .where((entry) => entry.pubkey == key)
                              .firstOrNull
                              ?.avatarUrl ??
                          directory.asData?.value
                              .where((entry) => entry.pubkey == key)
                              .firstOrNull
                              ?.avatarUrl,
                      location: remote != null ? 'Remote' : null,
                      onOpen: () => onSelect(key),
                      footer: editable
                          ? RemoteAgentModelControl(
                              key: ValueKey('model-$key'),
                              controller: controller,
                              agent: remote,
                              community: community,
                              isCurrent: () =>
                                  ref
                                      .read(activeCommunityProvider)
                                      .asData
                                      ?.value
                                      ?.id ==
                                  community.id,
                            )
                          : null,
                      subtitle: editable
                          ? null
                          : remote != null
                          ? 'Builderbot agent'
                          : 'Manage on desktop',
                    );
                  },
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}
