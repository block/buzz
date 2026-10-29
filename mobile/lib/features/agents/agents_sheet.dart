import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/me/default_agent.dart';
import '../../shared/agents/remote_agents.dart';
import '../../shared/community/community_provider.dart';
import '../../shared/hosted/hosted_flow.dart';
import '../../shared/hosted/hosted_flow_provider.dart';
import '../../shared/identity_names/identity_names_provider.dart';
import '../../shared/mentions/agent_identity_provider.dart';
import '../../shared/profile/user_cache_provider.dart';
import '../../shared/relay/relay.dart';
import '../../shared/theme/theme.dart';
import '../../shared/widgets/avatar_image.dart';
import '../../shared/widgets/frosted_app_bar.dart';
import '../../shared/widgets/frosted_scaffold.dart';
import '../../shared/theme/buzz_icons.dart';
import '../../shared/widgets/buzz_titled_sheet_layout.dart';
import '../../shared/widgets/modal_presentation.dart';
import '../channels/channel.dart';
import '../channels/channel_detail_page.dart';
import '../profile/user_profile_sheet.dart';
import 'create_agent_sheet.dart';
import 'delete_agent_action.dart';
import 'community_bot_profiles.dart';

/// Opens the active community's Agents navigation stack inside one modal.
Future<void> showAgents(BuildContext context) async {
  final container = ProviderScope.containerOf(context);
  final controller = container.read(remoteAgentControllerProvider);
  unawaited(controller.run(controller.refresh));
  final channel = await showBuzzModalBottomSheet<Channel>(
    context: context,
    isScrollControlled: true,
    showCloseButton: false,
    builder: (_) => _AgentsNavigation(controller: controller),
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
    return SizedBox(
      height: MediaQuery.sizeOf(context).height * .72,
      child: NavigatorPopHandler<Channel>(
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
              child: AgentsSheet(
                controller: controller,
                onClose: () => Navigator.of(context).pop(),
                onSelect: (pubkey) async {
                  final channel = await Navigator.of(listContext).push<Channel>(
                    MaterialPageRoute(
                      builder: (detailContext) => Material(
                        key: const ValueKey('agent-detail-surface'),
                        color: context.colors.surface,
                        child: SafeArea(
                          top: false,
                          bottom: false,
                          child: BuzzTitledSheetLayout(
                            title: '',
                            leading: IconButton(
                              tooltip: 'Back',
                              icon: const Icon(Icons.arrow_back_ios_new),
                              onPressed: () =>
                                  Navigator.of(detailContext).pop(),
                            ),
                            trailing: const SizedBox.shrink(),
                            child: UserProfileSheet(
                              pubkey: pubkey,
                              primaryActionsSpacing: Grid.md,
                              contextualActionsSpacing: Grid.xxs,
                              contextualActions: (_) => DeleteAgentAction(
                                controller: controller,
                                pubkey: pubkey,
                              ),
                            ),
                          ),
                        ),
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
      ),
    );
  }
}

/// The active community's agents, with remote-agent creation for its owner.
class AgentsSheet extends HookConsumerWidget {
  const AgentsSheet({
    required this.controller,
    required this.onClose,
    this.embedded = false,
    required this.onSelect,
    super.key,
  });
  final VoidCallback? onClose;
  final bool embedded;
  final ValueChanged<String> onSelect;
  final RemoteAgentController controller;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    useListenable(controller);
    final flow = ref.watch(hostedFlowProvider);
    useListenable(flow);
    final signingIn = flow.reconnecting && flow.stage != HostedFlowStage.idle;
    // Account sign-in belongs to the shared hosted flow; reload once it lands.
    useEffect(() {
      if (flow.reconnecting && flow.stage == HostedFlowStage.complete) {
        unawaited(controller.run(controller.refresh));
      }
      return null;
    }, [flow.stage]);
    final community = ref.watch(activeCommunityProvider).asData?.value;
    final directory = ref.watch(agentDirectoryProvider);
    final registryFuture = useMemoized(() async {
      final fetch = ref.read(relaySessionProvider.notifier).fetchHistory;
      return ownedCommunityAgents(
        {
          for (final agent in controller.agents)
            agent['agent_pubkey'] as String,
        },
        controller.owner,
        fetch,
      );
    }, [community?.relayUrl, controller.agents, controller.owner]);
    final registered = useFuture(registryFuture, preserveState: false);
    final keys = {
      ...?registered.data,
      ...?ref.watch(communityBotProfilesProvider).asData?.value,
      for (final entry in directory.asData?.value ?? <AgentDirectoryEntry>[])
        entry.pubkey,
    }.where((key) => !controller.deletedAgents.contains(key)).toList();
    final names = watchIdentityNames(
      ref,
      keys,
      agentPubkeys: keys.toSet(),
      fallbackNames: {
        for (final agent in controller.agents)
          agent['agent_pubkey'] as String: agent['agent_name'] as String,
      },
    );
    keys.sort((a, b) => names.labelFor(a).compareTo(names.labelFor(b)));
    final profiles = ref.watch(userCacheProvider);
    final creating = useState(false);
    final defaultAgent = ref.watch(defaultAgentProvider).asData?.value;

    final failed = directory.hasError || registered.hasError;
    final loading =
        !creating.value &&
        (controller.busy ||
            directory.isLoading ||
            registered.connectionState != ConnectionState.done);

    Future<void> refresh() async {
      await controller.run(controller.refresh);
      if (context.mounted) ref.invalidate(agentDirectoryProvider);
    }

    final headerTitleStyle = context.textTheme.titleMedium?.copyWith(
      fontSize: 22,
      fontWeight: FontWeight.w600,
      color: navigationPrimaryForeground(context),
    );
    final topSectionHeight = embedded
        ? frostedAppBarHeight(
            context,
            nativeLargeTitle: true,
            titleStyle: headerTitleStyle,
            bottomHeight: Grid.xxs,
          )
        : 0.0;
    final VoidCallback? addAgent =
        !supportsRemoteAgents(community) || controller.busy || creating.value
        ? null
        : () async {
            creating.value = true;
            try {
              await showBuzzModalBottomSheet<bool>(
                context: context,
                useRootNavigator: true,
                isScrollControlled: true,
                showCloseButton: false,
                builder: (_) => CreateAgentSheet(
                  controller: controller,
                  community: community!,
                  onCreated: () => ref.invalidate(agentDirectoryProvider),
                ),
              );
            } finally {
              if (context.mounted) creating.value = false;
            }
          };
    final content = Material(
      color: Colors.transparent,
      child: RefreshIndicator(
        onRefresh: refresh,
        child: ListView(
          physics: const AlwaysScrollableScrollPhysics(),
          padding: EdgeInsets.fromLTRB(
            Grid.xs,
            topSectionHeight,
            Grid.xs,
            Grid.sm,
          ),
          children: [
            if (loading) const LinearProgressIndicator(),
            const ListTile(
              subtitle: Text('Choose or change your Bestie on desktop.'),
            ),
            if (supportsRemoteAgents(community) &&
                controller.account.account == null)
              ListTile(
                title: const Text('Sign in to create an agent'),
                trailing: TextButton(
                  onPressed: controller.busy || signingIn
                      ? null
                      : () => unawaited(flow.reconnectAccount()),
                  child: const Text('Sign in'),
                ),
              ),
            if (!controller.busy &&
                controller.account.account != null &&
                controller.status.isNotEmpty &&
                !creating.value)
              ListTile(subtitle: Text(controller.status)),
            if (failed)
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
                  'No agents in this community yet.',
                  textAlign: TextAlign.center,
                ),
              ),
            for (final key in keys)
              ListTile(
                key: ValueKey(key),
                contentPadding: const EdgeInsets.symmetric(
                  horizontal: Grid.xxs,
                  vertical: Grid.xxs,
                ),
                leading: AvatarImage(
                  imageUrl: profiles[key]?.avatarUrl,
                  radius: Grid.sm,
                  isAgent: true,
                  backgroundColor: context.colors.surfaceContainerHighest,
                  fallback: Center(
                    child: Text(names.labelFor(key).characters.first),
                  ),
                ),
                title: Text(names.labelFor(key)),
                trailing: defaultAgent == key
                    ? const Tooltip(
                        message: 'Bestie · set on desktop',
                        child: Icon(Icons.star),
                      )
                    : null,
                onTap: () => onSelect(key),
              ),
          ],
        ),
      ),
    );
    if (embedded) {
      return FrostedScaffold(
        backgroundColor: context.colors.surface,
        appBar: FrostedAppBar(
          nativeTitle: 'Agents',
          nativeLargeTitle: true,
          automaticallyImplyLeading: false,
          nativeActions: [
            IosNavigationAction(
              label: 'Add agent',
              symbol: 'plus',
              icon: BuzzIcons.plus,
              onPressed: addAgent,
            ),
          ],
          horizontalInset: Grid.gutter,
          showBottomDivider: true,
          bottomDividerOpacity: 0.07,
          title: Text('Agents', style: headerTitleStyle),
          titleStyle: headerTitleStyle,
          actions: [
            IconButton(
              tooltip: 'Add agent',
              onPressed: addAgent,
              icon: const Icon(BuzzIcons.plus),
            ),
          ],
          bottomHeight: Grid.xxs,
          bottom: const SizedBox.expand(),
        ),
        body: content,
      );
    }
    return SizedBox(
      height: MediaQuery.sizeOf(context).height * .72,
      child: BuzzTitledSheetLayout(
        title: 'Agents',
        leading: IconButton(
          tooltip: 'Close',
          icon: const Icon(Icons.close),
          onPressed: onClose,
        ),
        trailing: IconButton(
          tooltip: 'Add agent',
          onPressed: addAgent,
          icon: const Icon(Icons.add),
        ),
        child: content,
      ),
    );
  }
}
