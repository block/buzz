import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/agents/owned_agent_directory.dart';
import '../../shared/widgets/bee_refresh_indicator.dart';
import '../../shared/widgets/buzz_toast.dart';
import '../../shared/agents/remote_agents.dart';
import '../../shared/deeplink/deep_link.dart';
import '../../shared/widgets/feedback_snackbar.dart';
import '../../shared/community/community_provider.dart';
import '../../shared/me/default_agent.dart';
import '../../shared/hosted/hosted_flow.dart';
import '../../shared/hosted/hosted_flow_provider.dart';
import '../../shared/identity_names/identity_names_provider.dart';
import '../../shared/mentions/agent_identity_provider.dart';
import '../../shared/profile/user_cache_provider.dart';
import '../../shared/relay/relay.dart';
import '../../shared/theme/theme.dart';
import '../../shared/widgets/modal_presentation.dart';
import '../../shared/widgets/agent_navigation_icon.dart';
import '../../shared/me/me_sync.dart';
import '../channels/bestie_welcome_storage.dart';
import '../channels/channel.dart';
import '../channels/channel_detail_page.dart';
import '../channels/channel_management_provider.dart';
import '../profile/user_profile_sheet.dart';
import 'agent_creation_draft.dart';
import 'create_agent_page.dart';
import 'agent_card.dart';
import 'remote_agent_model_control.dart';
import '../../shared/widgets/frosted_app_bar.dart';
import '../../shared/widgets/frosted_scaffold.dart';
import '../../shared/theme/buzz_icons.dart';
import 'agent_card_grid.dart';
import 'delete_agent_action.dart';
import 'agent_identity_sections.dart';
import 'bestie_agent_action.dart';

Route<Channel>? _openAgentsRoute;

/// The open Agents stack's nested navigator, so a link can tell whether
/// anything is open above its list.
GlobalKey<NavigatorState>? _openAgentsNavigator;

/// Opens the active community's full-page Agents navigation stack.
///
/// A [draft] from a `buzz://agent/new` link opens [CreateAgentPage] pre-filled
/// once the account, registry and community gates allow it.
Future<void> showAgents(
  BuildContext context, {
  AgentCreationDeepLink? draft,
  bool createOnOpen = false,
}) async {
  final container = ProviderScope.containerOf(context);
  final controller = container.read(remoteAgentControllerProvider);
  if (!context.mounted) return;
  final navigator = Navigator.of(context);
  // A link replaces an Agents stack that is already open instead of stacking
  // a second one on top of it, but only while it shows just its list: a form,
  // profile or dialog above it may hold work the user hasn't finished.
  final open = _openAgentsRoute;
  if (createOnOpen &&
      open != null &&
      open.isActive &&
      open.navigator == navigator) {
    return;
  }
  if (draft != null &&
      open != null &&
      open.isActive &&
      open.navigator == navigator) {
    if (!open.isCurrent ||
        (_openAgentsNavigator?.currentState?.canPop() ?? false)) {
      ScaffoldMessenger.maybeOf(context)?.showFeedbackSnackBar(
        const SnackBar(
          content: Text(
            'Finish or close what’s open in Agents, then reopen the link.',
          ),
        ),
        kind: ToastKind.error,
      );
      return;
    }
    navigator.removeRoute(open);
  }
  unawaited(controller.run(controller.refresh));
  final route = MaterialPageRoute<Channel>(
    builder: (_) => _AgentsNavigation(
      controller: controller,
      draft: draft,
      createOnOpen: createOnOpen,
    ),
  );
  _openAgentsRoute = route;
  final channel = await navigator.push(route);
  if (identical(_openAgentsRoute, route)) {
    _openAgentsRoute = null;
    _openAgentsNavigator = null;
  }
  // The caller's context can be gone by now: iOS Home drops the Bestie
  // shortcut while another route covers it. The navigator outlives it.
  if (channel != null && navigator.mounted) {
    await navigator.push<void>(
      MaterialPageRoute(
        // Native glass and video layers must track the live route.
        allowSnapshotting: false,
        builder: (_) => ChannelDetailPage(channel: channel),
      ),
    );
  }
}

class _AgentsNavigation extends HookConsumerWidget {
  const _AgentsNavigation({
    required this.controller,
    this.draft,
    this.createOnOpen = false,
  });

  final bool createOnOpen;

  final RemoteAgentController controller;
  final AgentCreationDeepLink? draft;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final communityId = ref.watch(activeCommunityProvider).asData?.value?.id;
    final heroController = useMemoized(HeroController.new);
    final navigator = useMemoized(() => GlobalKey<NavigatorState>(), [
      communityId,
    ]);
    if (identical(ModalRoute.of(context), _openAgentsRoute)) {
      _openAgentsNavigator = navigator;
    }
    return NavigatorPopHandler<Channel>(
      onPopWithResult: (result) {
        if (navigator.currentState?.canPop() ?? false) {
          navigator.currentState!.pop(result);
        }
      },
      child: Navigator(
        key: navigator,
        observers: [heroController],
        onGenerateRoute: (_) => MaterialPageRoute<void>(
          builder: (listContext) => Material(
            color: context.colors.surface,
            child: AgentsPage(
              controller: controller,
              draft: draft,
              createOnOpen: createOnOpen,
              onClose: () => Navigator.of(context).pop(),
              onCreated: (pubkey) async {
                // Only a first agent becomes Bestie; continue into its chat.
                final scope = ref.read(meSyncProvider);
                bool current() =>
                    context.mounted && ref.read(meSyncProvider) == scope;
                final Channel channel;
                try {
                  if (await ref.read(defaultAgentProvider.future) != pubkey ||
                      !current()) {
                    return;
                  }
                  channel = await ref
                      .read(channelActionsProvider)
                      .openDm(pubkeys: [pubkey]);
                } catch (error) {
                  if (context.mounted) {
                    ScaffoldMessenger.maybeOf(context)?.showFeedbackSnackBar(
                      SnackBar(
                        content: Text(
                          'Could not open the agent conversation. $error',
                        ),
                      ),
                      kind: ToastKind.error,
                    );
                  }
                  return;
                }
                if (!current()) return;
                // The welcome is local decoration; never let it block the chat.
                try {
                  await saveBestieWelcome(
                    ref.read(savedPrefsProvider),
                    scope,
                    channel.id,
                    pubkey,
                  );
                  ref.invalidate(bestieWelcomeProvider(channel.id));
                } catch (error) {
                  debugPrint('agents: Bestie welcome not saved: $error');
                }
                // Close only the Agents stack, never a route opened above it.
                if (!context.mounted || !current()) return;
                if (ModalRoute.of(context)?.isCurrent ?? false) {
                  Navigator.of(context).pop(channel);
                }
              },
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
                        : agentLocationUnknown(
                            ref.read(activeCommunityProvider).asData?.value,
                            controller,
                          )
                        ? null
                        : 'Manage on desktop',
                    primaryActionsSpacing: Grid.md,
                    contextualActions: (_) =>
                        BestieAgentAction(pubkey: pubkey, compact: true),
                    agentIdentity: (_) => AgentIdentitySections(
                      pubkey: pubkey,
                      controller: controller,
                    ),
                    agentSettings: (_) => DeleteAgentAction(
                      asSettingsRow: true,
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
    this.onCreated,
    this.draft,
    this.createOnOpen = false,
    super.key,
  });

  /// Link-supplied draft to open in [CreateAgentPage] when permitted.
  final AgentCreationDeepLink? draft;

  /// Open creation only after confirming there are no agents or saved setup.
  final bool createOnOpen;
  final VoidCallback onClose;
  final ValueChanged<String> onSelect;

  /// With [createOnOpen], called with the agent a creation here finished.
  /// The list stays busy until it completes.
  final Future<void> Function(String pubkey)? onCreated;
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
    final bestie = ref.watch(defaultAgentProvider).asData?.value;
    final ownedDirectory = ref.watch(ownedAgentDirectoryProvider);
    // The account's remote agents with a verified owner profile here are
    // listed too, so one stays listed while a deletion that already removed
    // its registration is retried. Every other agent comes from its listed
    // registration.
    final candidates = {
      if (controller.owner == community?.pubkey)
        for (final agent in controller.agents) agent['agent_pubkey'] as String,
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
    // Buzz App orders names without case and uses the identity as a stable tie.
    keys.sort((a, b) {
      final byName = names
          .labelFor(a)
          .toLowerCase()
          .compareTo(names.labelFor(b).toLowerCase());
      return byName != 0 ? byName : a.compareTo(b);
    });
    final profiles = ref.watch(userCacheProvider);
    final creating = useState(false);
    final needsSignIn = agentLocationUnknown(community, controller);
    final signInNeeded = builderlabSignInNeeded(community, controller);
    final failed =
        registered.hasError ||
        ownedDirectory.hasError ||
        // Remote listing failures matter only where remote agents are shown.
        (supportsRemoteAgents(community) && controller.listingFailed);
    final loading =
        !creating.value &&
        (controller.busy ||
            ownedDirectory.isLoading ||
            registered.connectionState != ConnectionState.done);

    Future<void> refresh() async {
      await controller.run(controller.refresh);
      if (context.mounted) {
        attempt.value++;
        ref.invalidate(ownedAgentDirectoryProvider);
      }
    }

    Future<void> openCreate([AgentCreationDraft? initialDraft]) async {
      if (community == null) return;
      creating.value = true;
      try {
        final done = await Navigator.of(context).push<bool>(
          MaterialPageRoute(
            builder: (_) => CreateAgentPage(
              controller: controller,
              community: community,
              initialDraft: initialDraft,
              onCreated: () {
                ref.invalidate(agentDirectoryProvider);
                ref.invalidate(ownedAgentDirectoryProvider);
              },
            ),
          ),
        );
        final agent = controller.created;
        if (createOnOpen && done == true && agent != null && context.mounted) {
          await onCreated?.call(agent);
        }
      } finally {
        if (context.mounted) creating.value = false;
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
            await openCreate();
          };

    // Open a creation request once, after sign-in and inventory loading.
    // The Bestie shortcut leaves existing agents and saved setups in the list.
    final draftStep = useRef(
      draft == null && !createOnOpen ? _DraftStep.done : _DraftStep.wait,
    );
    final draftRefreshFor = useRef<String?>(null);
    // Looked up during build: effects cannot read inherited widgets, and
    // snack bars cannot show while a frame is building.
    final messenger = ScaffoldMessenger.maybeOf(context);
    void drop(String message) {
      draftStep.value = _DraftStep.done;
      WidgetsBinding.instance.addPostFrameCallback((_) {
        messenger?.showFeedbackSnackBar(
          SnackBar(
            content: Text(
              draft == null
                  ? message
                        .replaceAll('Reopen the link', 'Reopen Agents')
                        .replaceAll('reopen the link', 'reopen Agents')
                  : message,
            ),
          ),
          kind: ToastKind.error,
        );
      });
    }

    useEffect(
      () {
        final link = draft;
        if (draftStep.value == _DraftStep.done) return null;
        if (link == null) {
          if (community == null || ownedDirectory.isLoading) return null;
          if (!supportsRemoteAgents(community) ||
              ownedDirectory.hasError ||
              (ownedDirectory.value?.isNotEmpty ?? false)) {
            draftStep.value = _DraftStep.done;
            return null;
          }
        }
        // Sign-in counts as attempted only once it is seen running, so a
        // rerun before reconnectAccount reaches its sign-in stage still waits.
        if (signingIn && draftStep.value == _DraftStep.signInRequested) {
          draftStep.value = _DraftStep.signingIn;
        }
        if (community == null || controller.busy || signingIn) return null;
        if (!supportsRemoteAgents(community)) {
          drop('Remote agents aren’t available in this community.');
          return null;
        }
        if (controller.account.account == null) {
          if (draftStep.value == _DraftStep.wait) {
            draftStep.value = _DraftStep.signInRequested;
            unawaited(
              flow.reconnectAccount().catchError((Object error) {
                debugPrint('agents: link sign-in failed: $error');
                if (context.mounted) {
                  drop('Couldn’t start Builderlab sign-in. Reopen the link.');
                }
              }),
            );
          } else if (draftStep.value == _DraftStep.signingIn) {
            drop(
              'Sign in to Builderlab to create this agent. Reopen the link.',
            );
          }
          return null;
        }
        // The setup check below needs this account's registry, which a
        // sign-in started by the link doesn't reload by itself.
        final subject = controller.account.account!.subject;
        if (!controller.loadedForCurrentAccount) {
          if (draftRefreshFor.value != subject) {
            draftRefreshFor.value = subject;
            unawaited(controller.run(controller.refresh));
          } else {
            drop('Couldn’t load your agents. Reopen the link to retry.');
          }
          return null;
        }
        if (controller.listingFailed) {
          drop('Couldn’t load your agents. Reopen the link to retry.');
          return null;
        }
        if (controller.pending != null) {
          if (link == null) {
            draftStep.value = _DraftStep.done;
          } else {
            drop(
              'Finish your unfinished agent setup first, then reopen the link.',
            );
          }
          return null;
        }
        if (link == null) {
          if (registered.connectionState != ConnectionState.done) return null;
          if (registered.hasError || keys.isNotEmpty) {
            draftStep.value = _DraftStep.done;
            return null;
          }
          draftStep.value = _DraftStep.done;
          WidgetsBinding.instance.addPostFrameCallback((_) {
            if (context.mounted) unawaited(openCreate());
          });
          return null;
        }
        draftStep.value = _DraftStep.done;
        // The checks above hold for this account; the fetch can take minutes.
        final client = ref.read(agentCardClientProvider)();
        unawaited(() async {
          // Hold Create (+) off while the card loads, like an open form does.
          // Deferred past this build: hooks state can't change mid-build.
          await Future<void>.value();
          if (!context.mounted) return client.close();
          creating.value = true;
          final AgentCreationDraft resolved;
          try {
            resolved = await AgentCreationDraft.fetch(link, client);
          } catch (error) {
            if (error is! AgentCardException) {
              debugPrint('agents: agent card failed: $error');
            }
            if (context.mounted) {
              creating.value = false;
              drop(
                error is AgentCardException
                    ? error.message
                    : 'Couldn’t open this agent card.',
              );
            }
            return;
          } finally {
            client.close();
          }
          if (!context.mounted) return;
          if (controller.account.account?.subject != subject) {
            creating.value = false;
            drop('Your Builderlab account changed. Reopen the link.');
          } else if (controller.pending != null) {
            creating.value = false;
            drop(
              'Finish your unfinished agent setup first, then reopen the link.',
            );
          } else {
            await openCreate(resolved);
          }
        }());
        return null;
      },
      [
        controller.busy,
        signingIn,
        flow.stage,
        community?.id,
        controller.account.account?.subject,
        controller.loadedForCurrentAccount,
        ownedDirectory,
        registered.connectionState,
        candidateKey,
      ],
    );

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
          child: BeeRefreshIndicator(
            edgeOffset: topHeight,
            onRefresh: refresh,
            child: CustomScrollView(
              physics: const AlwaysScrollableScrollPhysics(),
              slivers: [
                SliverToBoxAdapter(child: SizedBox(height: topHeight)),
                if (signInNeeded)
                  SliverToBoxAdapter(
                    child: ReconnectCapsule(
                      inline: true,
                      message: signingIn
                          ? 'Finish sign-in in browser'
                          : 'Reconnect to Builderlab',
                      actionLabel: signingIn ? 'Cancel' : 'Sign in',
                      destructiveAction: signingIn,
                      onSignIn: () => unawaited(
                        signingIn ? flow.leave() : flow.reconnectAccount(),
                      ),
                    ),
                  ),
                SliverToBoxAdapter(
                  child: Column(
                    children: [
                      if (createOnOpen && (loading || signingIn))
                        const LinearProgressIndicator(),
                      if (createOnOpen &&
                          supportsRemoteAgents(community) &&
                          controller.failure.isNotEmpty)
                        Padding(
                          padding: const EdgeInsets.all(Grid.sm),
                          child: Text(controller.failure),
                        ),
                      if (failed && keys.isEmpty)
                        ListTile(
                          title: const Text('Could not load agents'),
                          trailing: TextButton(
                            onPressed: refresh,
                            child: const Text('Retry'),
                          ),
                        ),
                      if (!loading && !failed && !needsSignIn && keys.isEmpty)
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
                      isBestie: key == bestie,
                      imageUrl:
                          profiles[key]?.avatarUrl ??
                          ownedDirectory.asData?.value
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
                          : needsSignIn
                          ? null
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

/// Progress of opening a link's draft: waiting on gates, sign-in requested,
/// sign-in seen running, or done.
enum _DraftStep { wait, signInRequested, signingIn, done }
