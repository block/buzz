import 'dart:math' as math;

import 'package:flutter/cupertino.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';

import '../../../shared/theme/theme.dart';
import '../../../shared/utils/string_utils.dart';
import '../../../shared/widgets/buzz_loading_indicator.dart';
import '../../../shared/widgets/frosted_app_bar.dart';
import '../../../shared/profile/user_cache_provider.dart';
import '../../../shared/profile/user_profile.dart';
import '../channel_typing_indicator.dart';
import '../small_avatar.dart';
import 'active_agent_turns.dart';
import 'observer_models.dart';
import 'observer_subscription.dart';
import 'working_bots_provider.dart';

part 'composer_agent_activity_indicator/agent_activity_controls.dart';
part 'composer_agent_activity_indicator/compact_activity_item.dart';

/// Composer-adjacent agent status that expands upward into a bounded activity
/// stream without moving or covering the composer itself.
class ComposerAgentActivityIndicator extends HookConsumerWidget {
  static const _compactSurfaceHeight = 52.0;
  static const _baseExpandedTargetHeight = 328.0;
  static const _surfaceMorphDuration = Duration(milliseconds: 220);
  static const _surfaceMorphCurve = Cubic(0.77, 0, 0.175, 1);

  final String channelId;
  final String? threadHeadId;
  final bool animated;
  final double horizontalInset;
  final double? overlayTopBoundary;
  final double compactWidthFactor;
  final Animation<double>? composerWidthAnimation;
  final FocusNode? composerFocusNode;
  final ValueNotifier<bool>? composerInteractionLock;
  final ValueNotifier<int>? composerActivationRequests;
  final VoidCallback? onRestoreComposerFocus;

  const ComposerAgentActivityIndicator({
    super.key,
    required this.channelId,
    this.threadHeadId,
    this.animated = true,
    this.horizontalInset = Grid.twelve,
    this.overlayTopBoundary,
    this.compactWidthFactor = 1,
    this.composerWidthAnimation,
    this.composerFocusNode,
    this.composerInteractionLock,
    this.composerActivationRequests,
    this.onRestoreComposerFocus,
  }) : assert(compactWidthFactor > 0 && compactWidthFactor <= 1);

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final activity = ref.watch(
      composerActivityStateProvider((
        channelId: channelId,
        threadHeadId: threadHeadId,
      )),
    );
    final turnStates = ref.watch(agentTurnStatesProvider);
    final profiles = ref.watch(userCacheProvider);
    final expanded = useState(false);
    final pendingExpansion = useState(false);
    final selectedAgent = useState<String?>(null);
    final pinnedTurnId = useState<String?>(null);
    final rememberedAgents = useState<List<String>>(const []);
    final activityAnchorKey = useMemoized(GlobalKey.new);
    final activityOverlayController = useMemoized(OverlayPortalController.new);
    final compactActivityWidth = useState(0.0);
    final expandedHeight = useState(_baseExpandedTargetHeight);
    final expansionController = useAnimationController(
      duration: _surfaceMorphDuration,
    );
    final scrollController = useScrollController();
    final followsTail = useRef(true);
    final composerFocusToRestore = useRef<FocusNode?>(null);
    final restoreComposerFocus = useRef(false);
    final previousTranscriptVersion = useRef('');
    final reducedMotion = MediaQuery.disableAnimationsOf(context);
    final motionDisabled = !animated || reducedMotion;
    final mediaSize = MediaQuery.sizeOf(context);
    final expandedTargetHeight = _expandedPanelTargetHeight(context);
    final viewInsets = MediaQuery.viewInsetsOf(context);
    final viewPadding = MediaQuery.viewPaddingOf(context);
    final platformView = View.of(context);
    final keyboardInsetBottom = math.max(
      viewInsets.bottom,
      platformView.viewInsets.bottom / platformView.devicePixelRatio,
    );
    final widthAnimation = composerWidthAnimation ?? kAlwaysDismissedAnimation;
    useListenable(widthAnimation);
    final composerWidthProgress = widthAnimation.value
        .clamp(0.0, 1.0)
        .toDouble();

    void updateOverlayGeometry() {
      final renderObject = activityAnchorKey.currentContext?.findRenderObject();
      if (renderObject is! RenderBox || !renderObject.hasSize) return;
      final anchorBottom = renderObject
          .localToGlobal(Offset(0, renderObject.size.height))
          .dy;
      final topBoundary = overlayTopBoundary ?? frostedAppBarHeight(context);
      final availableHeight = math.max(
        _compactSurfaceHeight,
        anchorBottom - topBoundary - Grid.xxs,
      );
      final nextExpandedHeight = math.min(
        expandedTargetHeight,
        availableHeight,
      );
      final nextCompactWidth = renderObject.size.width;
      if ((compactActivityWidth.value - nextCompactWidth).abs() >= 0.5) {
        compactActivityWidth.value = nextCompactWidth;
      }
      if ((expandedHeight.value - nextExpandedHeight).abs() >= 0.5) {
        expandedHeight.value = nextExpandedHeight;
      }
    }

    void releaseComposerFocusLock() {
      final interactionLock = composerInteractionLock;
      if (interactionLock != null && interactionLock.value) {
        interactionLock.value = false;
      }
    }

    void restoreComposerAfterCollapse() {
      final focusNode = composerFocusToRestore.value;
      final shouldRestore = restoreComposerFocus.value;
      composerFocusToRestore.value = null;
      restoreComposerFocus.value = false;
      releaseComposerFocusLock();
      if (!shouldRestore || focusNode == null) return;
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (!context.mounted) return;
        if (onRestoreComposerFocus != null) {
          onRestoreComposerFocus!();
        } else if (focusNode.canRequestFocus) {
          focusNode.requestFocus();
        }
      });
    }

    useEffect(() {
      return () {
        final interactionLock = composerInteractionLock;
        if (interactionLock != null && interactionLock.value) {
          interactionLock.value = false;
        }
      };
    }, [composerInteractionLock]);

    void hideActivityOverlay() {
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (context.mounted && activityOverlayController.isShowing) {
          activityOverlayController.hide();
        }
      });
    }

    useEffect(
      () {
        WidgetsBinding.instance.addPostFrameCallback((_) {
          if (context.mounted) updateOverlayGeometry();
        });
        return null;
      },
      [
        mediaSize.height,
        keyboardInsetBottom,
        viewPadding.top,
        overlayTopBoundary,
        expandedTargetHeight,
        expanded.value,
      ],
    );

    useEffect(() {
      if (!pendingExpansion.value ||
          keyboardInsetBottom > 0.5 ||
          composerWidthProgress > 0.001) {
        return null;
      }
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (!context.mounted || !pendingExpansion.value) return;
        updateOverlayGeometry();
        activityOverlayController.show();
        pendingExpansion.value = false;
        expanded.value = true;
      });
      return null;
    }, [pendingExpansion.value, keyboardInsetBottom, composerWidthProgress]);

    useEffect(() {
      if (motionDisabled) {
        expansionController.value = expanded.value ? 1 : 0;
      } else if (expanded.value) {
        expansionController.forward();
      } else {
        expansionController.reverse();
      }
      return null;
    }, [expanded.value, motionDisabled, expansionController]);

    useEffect(() {
      void clearSelectionAfterCollapse(AnimationStatus status) {
        if (status != AnimationStatus.dismissed || expanded.value) return;
        selectedAgent.value = null;
        pinnedTurnId.value = null;
        rememberedAgents.value = const [];
        hideActivityOverlay();
        WidgetsBinding.instance.addPostFrameCallback((_) {
          if (!context.mounted ||
              expanded.value ||
              !expansionController.isDismissed) {
            return;
          }
          restoreComposerAfterCollapse();
        });
      }

      expansionController.addStatusListener(clearSelectionAfterCollapse);
      return () =>
          expansionController.removeStatusListener(clearSelectionAfterCollapse);
    }, [expansionController]);

    final viewableWorking = [
      for (final signal in activity.agents)
        if (signal.canViewActivity) signal,
    ];
    final statusOnlyAgents = [
      for (final signal in activity.agents)
        if (!signal.canViewActivity) signal,
    ];
    final effectiveSelectedAgent =
        selectedAgent.value ?? viewableWorking.firstOrNull?.pubkey;
    final selectedSignal = viewableWorking
        .where((signal) => signal.pubkey == effectiveSelectedAgent)
        .firstOrNull;
    final effectiveTurnId = pinnedTurnId.value ?? selectedSignal?.turnId;
    final selectedTurn =
        effectiveSelectedAgent == null || effectiveTurnId == null
        ? null
        : latestAgentTurnState(
            turnStates,
            agentPubkey: effectiveSelectedAgent,
            channelId: channelId,
            threadHeadId: threadHeadId,
            turnId: effectiveTurnId,
          );
    final ObserverState? observerState;
    if (effectiveSelectedAgent == null || effectiveTurnId == null) {
      observerState = null;
    } else {
      observerState = ref.watch(
        observerTurnSubscriptionProvider((
          channelId: channelId,
          agentPubkey: effectiveSelectedAgent,
          turnId: effectiveTurnId,
        )),
      );
    }
    final transcript = compactActivityItems(
      observerState?.transcript ?? const [],
    );

    String nameFor(String pubkey) =>
        profiles[pubkey.toLowerCase()]?.label ?? shortPubkey(pubkey);

    useEffect(() {
      final pubkeys = {
        ...activity.agents.map((signal) => signal.pubkey),
        ?effectiveSelectedAgent,
      };
      if (pubkeys.isNotEmpty) {
        ref.read(userCacheProvider.notifier).preload(pubkeys.toList());
      }
      return null;
    }, [activity.agents, effectiveSelectedAgent]);

    useEffect(() {
      final turnId = selectedSignal?.turnId;
      if (!expanded.value || pinnedTurnId.value != null || turnId == null) {
        return null;
      }
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (context.mounted && expanded.value && pinnedTurnId.value == null) {
          pinnedTurnId.value = turnId;
        }
      });
      return null;
    }, [expanded.value, selectedSignal?.turnId]);

    final transcriptVersion = _transcriptVersion(transcript);
    useEffect(() {
      if (transcriptVersion == previousTranscriptVersion.value) return null;
      previousTranscriptVersion.value = transcriptVersion;
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (scrollController.hasClients && followsTail.value) {
          scrollController.jumpTo(scrollController.position.maxScrollExtent);
        }
      });
      return null;
    }, [transcriptVersion, effectiveSelectedAgent]);

    void collapse() {
      pendingExpansion.value = false;
      expanded.value = false;
      followsTail.value = true;
      if (expansionController.isDismissed) {
        selectedAgent.value = null;
        pinnedTurnId.value = null;
        rememberedAgents.value = const [];
        hideActivityOverlay();
        restoreComposerAfterCollapse();
      }
    }

    useEffect(
      () {
        final requests = composerActivationRequests;
        if (requests == null) return null;
        void handOffToComposer() {
          if (!expanded.value && !pendingExpansion.value) return;
          if (composerInteractionLock != null) {
            composerInteractionLock!.value = true;
          }
          final focusNode = composerFocusNode;
          if (focusNode != null) {
            restoreComposerFocus.value = true;
            composerFocusToRestore.value = focusNode;
          }
          collapse();
        }

        requests.addListener(handOffToComposer);
        return () => requests.removeListener(handOffToComposer);
      },
      [
        composerActivationRequests,
        composerFocusNode,
        expanded.value,
        pendingExpansion.value,
      ],
    );

    void selectAgent(String pubkey) {
      final signal = viewableWorking
          .where((candidate) => candidate.pubkey == pubkey)
          .firstOrNull;
      final latestTurn = latestAgentTurnState(
        turnStates,
        agentPubkey: pubkey,
        channelId: channelId,
        threadHeadId: threadHeadId,
      );
      selectedAgent.value = pubkey;
      pinnedTurnId.value = signal?.turnId ?? latestTurn?.turnId;
      followsTail.value = true;
    }

    void toggleExpanded() {
      if (expanded.value || pendingExpansion.value) {
        collapse();
        return;
      }
      final initial = effectiveSelectedAgent;
      if (initial == null) return;
      selectedAgent.value = initial;
      pinnedTurnId.value = selectedSignal?.turnId;
      rememberedAgents.value = [
        for (final signal in viewableWorking) signal.pubkey,
      ];
      followsTail.value = true;
      final wasKeyboardOpen = keyboardInsetBottom > 0.5;
      final focusedNode =
          composerFocusNode ?? FocusManager.instance.primaryFocus;
      restoreComposerFocus.value =
          wasKeyboardOpen && focusedNode?.hasFocus == true;
      composerFocusToRestore.value = restoreComposerFocus.value
          ? focusedNode
          : null;
      if (composerInteractionLock != null) {
        composerInteractionLock!.value = true;
      }
      if (composerFocusNode != null) {
        composerFocusNode!.unfocus(
          disposition: UnfocusDisposition.previouslyFocusedChild,
        );
      } else if (focusedNode != null) {
        focusedNode.unfocus(
          disposition: UnfocusDisposition.previouslyFocusedChild,
        );
      }
      if (wasKeyboardOpen || composerWidthProgress > 0.001) {
        pendingExpansion.value = true;
      } else {
        updateOverlayGeometry();
        activityOverlayController.show();
        expanded.value = true;
      }
    }

    final selectorAgents = <String>{
      ...rememberedAgents.value,
      ...viewableWorking.map((signal) => signal.pubkey),
      ?effectiveSelectedAgent,
    }.toList();
    final hasContent =
        expanded.value ||
        rememberedAgents.value.isNotEmpty ||
        activity.agents.isNotEmpty ||
        activity.humanTyping.isNotEmpty;
    final child = hasContent
        ? Column(
            key: const ValueKey('composer-activity-region'),
            mainAxisSize: MainAxisSize.min,
            children: [
              if ((viewableWorking.isNotEmpty ||
                      rememberedAgents.value.isNotEmpty) &&
                  effectiveSelectedAgent != null)
                OverlayPortal.overlayChildLayoutBuilder(
                  controller: activityOverlayController,
                  overlayChildBuilder: (context, layoutInfo) {
                    final anchorOrigin = MatrixUtils.transformPoint(
                      layoutInfo.childPaintTransform,
                      Offset.zero,
                    );
                    final availableWidth = math.max(
                      layoutInfo.childSize.width,
                      layoutInfo.overlaySize.width - Grid.twelve * 2,
                    );
                    final expandedActivityWidth = math.max(
                      layoutInfo.childSize.width,
                      math.min(
                        availableWidth,
                        layoutInfo.childSize.width / compactWidthFactor,
                      ),
                    );
                    return Stack(
                      children: [
                        AnimatedBuilder(
                          animation: expansionController,
                          builder: (context, _) {
                            final progress = _surfaceMorphCurve.transform(
                              expansionController.value,
                            );
                            final panelViewportHeight =
                                _compactSurfaceHeight +
                                (expandedHeight.value - _compactSurfaceHeight) *
                                    progress;
                            final panelWidth =
                                compactActivityWidth.value +
                                (expandedActivityWidth -
                                        compactActivityWidth.value) *
                                    progress;
                            if (panelWidth <= 0) {
                              return const SizedBox.shrink();
                            }
                            return Positioned(
                              left:
                                  (layoutInfo.overlaySize.width - panelWidth) /
                                  2,
                              bottom:
                                  layoutInfo.overlaySize.height -
                                  anchorOrigin.dy -
                                  layoutInfo.childSize.height,
                              child: SizedBox(
                                key: const ValueKey(
                                  'composer-agent-activity-overlay',
                                ),
                                width: panelWidth,
                                height: panelViewportHeight,
                                child: IgnorePointer(
                                  ignoring: expansionController.isDismissed,
                                  child: ExcludeSemantics(
                                    excluding: expansionController.isDismissed,
                                    child: ClipRect(
                                      child: _InlineActivityPanel(
                                        height: panelViewportHeight,
                                        panelWidth: panelWidth,
                                        expandedHeight: expandedHeight.value,
                                        morphProgress: progress,
                                        targetExpanded: expanded.value,
                                        agentName: nameFor(
                                          effectiveSelectedAgent,
                                        ),
                                        selectedTurn: selectedTurn,
                                        isFallbackWorking:
                                            selectedSignal != null,
                                        observerState: observerState,
                                        transcript: transcript,
                                        profiles: profiles,
                                        signals: viewableWorking,
                                        selectorAgents: selectorAgents,
                                        selectedAgent: effectiveSelectedAgent,
                                        nameFor: nameFor,
                                        onSelectAgent: selectAgent,
                                        onToggleExpanded: toggleExpanded,
                                        horizontalInset: horizontalInset,
                                        scrollController: scrollController,
                                        onScroll: (notification) {
                                          if (notification
                                                  is ScrollUpdateNotification &&
                                              notification.dragDetails !=
                                                  null) {
                                            followsTail.value =
                                                notification
                                                        .metrics
                                                        .maxScrollExtent -
                                                    notification
                                                        .metrics
                                                        .pixels <=
                                                24;
                                          }
                                          return false;
                                        },
                                      ),
                                    ),
                                  ),
                                ),
                              ),
                            );
                          },
                        ),
                      ],
                    );
                  },
                  child: SizedBox(
                    key: activityAnchorKey,
                    width: double.infinity,
                    height: _compactSurfaceHeight,
                    child: Semantics(
                      key: const ValueKey('composer-agent-activity-surface'),
                      container: true,
                      child: AnimatedBuilder(
                        animation: expansionController,
                        builder: (context, _) {
                          final progress = _surfaceMorphCurve.transform(
                            expansionController.value,
                          );
                          if (progress >= 1 || viewableWorking.isEmpty) {
                            return const SizedBox.expand();
                          }
                          return Opacity(
                            opacity: 1 - progress,
                            child: IgnorePointer(
                              ignoring:
                                  expanded.value ||
                                  !expansionController.isDismissed,
                              child: _AgentActivityControl(
                                signals: viewableWorking,
                                selectedAgent: effectiveSelectedAgent,
                                selectedTurn: selectedTurn,
                                transcript: transcript,
                                profiles: profiles,
                                nameFor: nameFor,
                                onTap: toggleExpanded,
                                horizontalInset: horizontalInset,
                              ),
                            ),
                          );
                        },
                      ),
                    ),
                  ),
                ),
              if (statusOnlyAgents.isNotEmpty)
                _AgentStatusRow(
                  signals: statusOnlyAgents,
                  profiles: profiles,
                  nameFor: nameFor,
                  horizontalInset: horizontalInset,
                ),
              if (activity.humanTyping.isNotEmpty)
                ChannelTypingIndicator(entries: activity.humanTyping),
            ],
          )
        : const SizedBox.shrink();
    if (motionDisabled) return child;
    return AnimatedSize(
      duration: _surfaceMorphDuration,
      curve: _surfaceMorphCurve,
      alignment: Alignment.bottomCenter,
      child: child,
    );
  }
}

@visibleForTesting
List<TranscriptItem> compactActivityItems(List<TranscriptItem> transcript) {
  final useful = <TranscriptItem>[];
  for (final item in transcript) {
    if (item is MetadataItem) continue;
    if (item is LifecycleItem &&
        (item.title == 'Turn started' || item.title == 'Session ready')) {
      continue;
    }
    useful.add(item);
  }
  return List.unmodifiable(
    useful.length <= 40 ? useful : useful.sublist(useful.length - 40),
  );
}

class _InlineActivityPanel extends StatelessWidget {
  final double height;
  final double panelWidth;
  final double expandedHeight;
  final double morphProgress;
  final bool targetExpanded;
  final String agentName;
  final AgentTurnState? selectedTurn;
  final bool isFallbackWorking;
  final ObserverState? observerState;
  final List<TranscriptItem> transcript;
  final Map<String, UserProfile> profiles;
  final List<WorkingAgentSignal> signals;
  final List<String> selectorAgents;
  final String selectedAgent;
  final String Function(String) nameFor;
  final ValueChanged<String> onSelectAgent;
  final VoidCallback onToggleExpanded;
  final double horizontalInset;
  final ScrollController scrollController;
  final bool Function(ScrollNotification) onScroll;

  const _InlineActivityPanel({
    required this.height,
    required this.panelWidth,
    required this.expandedHeight,
    required this.morphProgress,
    required this.targetExpanded,
    required this.agentName,
    required this.selectedTurn,
    required this.isFallbackWorking,
    required this.observerState,
    required this.transcript,
    required this.profiles,
    required this.signals,
    required this.selectorAgents,
    required this.selectedAgent,
    required this.nameFor,
    required this.onSelectAgent,
    required this.onToggleExpanded,
    required this.horizontalInset,
    required this.scrollController,
    required this.onScroll,
  });

  @override
  Widget build(BuildContext context) {
    final status = _activityStatus(selectedTurn, isFallbackWorking);
    final headline = _selectedActivityHeadline(selectedTurn, transcript);
    final compactLabel = _agentActivityLabel(
      pubkeys: selectorAgents,
      signals: signals,
      selectedTurn: selectedTurn,
      transcript: transcript,
      nameFor: nameFor,
      expanded: false,
    ).visibleLabel;
    final topInset = Grid.xxs * morphProgress;
    final labelStyle = context.textTheme.labelSmall?.copyWith(
      fontWeight: FontWeight.w600,
    );
    final statusLabel = _activityStatusLabel(status);
    final contentWidth = math.max(
      0.0,
      panelWidth - horizontalInset * 2 - Grid.xxs * 2,
    );
    final inlineFooterWidth =
        _agentAvatarStackWidth(selectorAgents.length) +
        Grid.xxs +
        _singleLineTextWidth(context, 'Live activity', labelStyle) +
        Grid.xxs +
        _singleLineTextWidth(context, statusLabel, labelStyle) +
        Grid.xxs * 2 +
        Grid.half +
        18;
    final stacksFooter = inlineFooterWidth > contentWidth;
    final footerHeight = _activityFooterHeight(context, stacked: stacksFooter);
    final panelHeight = math.max(footerHeight, height - Grid.xxs - topInset);
    final expandedDetailHeight = math.max(
      0.0,
      expandedHeight - Grid.xxs * 2 - footerHeight,
    );
    final detailsOpacity = Curves.easeOut.transform(
      ((morphProgress - 0.16) / 0.84).clamp(0.0, 1.0),
    );

    return Padding(
      padding: EdgeInsets.fromLTRB(
        0,
        topInset,
        0,
        Grid.xxs,
      ).add(EdgeInsets.symmetric(horizontal: horizontalInset)),
      child: Semantics(
        container: true,
        label: 'Live activity for $agentName',
        child: Material(
          key: const ValueKey('composer-agent-activity-panel'),
          color: context.colors.surfaceContainerHighest,
          shape: RoundedRectangleBorder(
            borderRadius: BorderRadius.circular(Radii.dialog),
            side: BorderSide(color: Colors.black.withValues(alpha: 0.04)),
          ),
          clipBehavior: Clip.antiAlias,
          child: SizedBox(
            height: panelHeight,
            width: double.infinity,
            child: Stack(
              children: [
                Positioned.fill(
                  bottom: footerHeight,
                  child: ClipRect(
                    child: IgnorePointer(
                      ignoring: morphProgress < 0.95,
                      child: Opacity(
                        key: const ValueKey('composer-agent-activity-details'),
                        opacity: detailsOpacity,
                        child: OverflowBox(
                          alignment: Alignment.topCenter,
                          minHeight: expandedDetailHeight,
                          maxHeight: expandedDetailHeight,
                          child: Column(
                            children: [
                              if (selectorAgents.length > 1)
                                _AgentSegmentedControl(
                                  agents: selectorAgents,
                                  selectedAgent: selectedAgent,
                                  nameFor: nameFor,
                                  onSelectAgent: onSelectAgent,
                                )
                              else
                                const SizedBox(height: Grid.xxs),
                              Padding(
                                padding: const EdgeInsets.symmetric(
                                  horizontal: Grid.xxs,
                                ),
                                child: Align(
                                  alignment: Alignment.centerLeft,
                                  child: Text(
                                    'Live activity may be partial.',
                                    maxLines: 2,
                                    overflow: TextOverflow.ellipsis,
                                    style: context.textTheme.labelSmall
                                        ?.copyWith(
                                          color:
                                              context.colors.onSurfaceVariant,
                                        ),
                                  ),
                                ),
                              ),
                              Expanded(
                                child: transcript.isEmpty
                                    ? _ActivityEmptyState(
                                        status: status,
                                        observerState: observerState,
                                      )
                                    : NotificationListener<ScrollNotification>(
                                        onNotification: onScroll,
                                        child: ListView.builder(
                                          key: const ValueKey(
                                            'composer-agent-activity-transcript',
                                          ),
                                          controller: scrollController,
                                          padding: const EdgeInsets.fromLTRB(
                                            Grid.xxs,
                                            Grid.half,
                                            Grid.xxs,
                                            Grid.xxs,
                                          ),
                                          itemCount: transcript.length,
                                          itemBuilder: (context, index) =>
                                              _CompactActivityItem(
                                                item: transcript[index],
                                              ),
                                        ),
                                      ),
                              ),
                            ],
                          ),
                        ),
                      ),
                    ),
                  ),
                ),
                Positioned(
                  left: 0,
                  right: 0,
                  bottom: 0,
                  height: footerHeight,
                  child: Semantics(
                    button: true,
                    label:
                        '$agentName $headline. ${targetExpanded ? 'Collapse' : 'Expand'} live activity.',
                    child: ExcludeSemantics(
                      child: InkWell(
                        key: const ValueKey('composer-agent-activity-collapse'),
                        onTap: onToggleExpanded,
                        child: Padding(
                          padding: const EdgeInsets.symmetric(
                            horizontal: Grid.xxs,
                            vertical: Grid.half,
                          ),
                          child: stacksFooter
                              ? Column(
                                  mainAxisAlignment: MainAxisAlignment.center,
                                  children: [
                                    Row(
                                      children: [
                                        _AgentAvatarStack(
                                          pubkeys: selectorAgents,
                                          profiles: profiles,
                                        ),
                                        const SizedBox(width: Grid.xxs),
                                        _ActivityFooterLabel(
                                          compactLabel: compactLabel,
                                          morphProgress: morphProgress,
                                        ),
                                        Transform.rotate(
                                          angle: math.pi * morphProgress,
                                          child: Icon(
                                            LucideIcons.chevronUp,
                                            size: 18,
                                            color:
                                                context.colors.onSurfaceVariant,
                                          ),
                                        ),
                                      ],
                                    ),
                                    const SizedBox(height: Grid.half),
                                    Align(
                                      alignment: Alignment.centerLeft,
                                      child: _ActivityStatusBadge(
                                        status: status,
                                      ),
                                    ),
                                  ],
                                )
                              : Row(
                                  children: [
                                    _AgentAvatarStack(
                                      pubkeys: selectorAgents,
                                      profiles: profiles,
                                    ),
                                    const SizedBox(width: Grid.xxs),
                                    _ActivityFooterLabel(
                                      compactLabel: compactLabel,
                                      morphProgress: morphProgress,
                                    ),
                                    ClipRect(
                                      child: Align(
                                        widthFactor: morphProgress,
                                        child: Opacity(
                                          opacity: morphProgress,
                                          child: _ActivityStatusBadge(
                                            status: status,
                                          ),
                                        ),
                                      ),
                                    ),
                                    SizedBox(width: Grid.half * morphProgress),
                                    Transform.rotate(
                                      angle: math.pi * morphProgress,
                                      child: Icon(
                                        LucideIcons.chevronUp,
                                        size: 18,
                                        color: context.colors.onSurfaceVariant,
                                      ),
                                    ),
                                  ],
                                ),
                        ),
                      ),
                    ),
                  ),
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}

class _ActivityEmptyState extends StatelessWidget {
  final _ActivityStatus status;
  final ObserverState? observerState;

  const _ActivityEmptyState({
    required this.status,
    required this.observerState,
  });

  @override
  Widget build(BuildContext context) {
    if (status == _ActivityStatus.error) {
      return Center(
        child: Text(
          'The agent stopped with an error.',
          style: context.textTheme.bodySmall?.copyWith(
            color: context.colors.error,
          ),
        ),
      );
    }
    if (observerState?.connection == ObserverConnectionState.error) {
      return Center(
        child: Text(
          'Live activity is unavailable.',
          style: context.textTheme.bodySmall?.copyWith(
            color: context.colors.error,
          ),
        ),
      );
    }
    return Center(
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          if (status == _ActivityStatus.working ||
              status == _ActivityStatus.waiting)
            BuzzLoadingIndicator(
              size: 18,
              color: context.colors.onSurfaceVariant,
              semanticLabel: 'Waiting for agent activity',
            ),
          const SizedBox(height: Grid.half),
          Text(
            switch (status) {
              _ActivityStatus.finished =>
                'No activity rows were captured for this turn.',
              _ActivityStatus.cancelled => 'This turn was cancelled.',
              _ => 'Waiting for live activity…',
            },
            style: context.textTheme.bodySmall?.copyWith(
              color: context.colors.onSurfaceVariant,
            ),
            textAlign: TextAlign.center,
          ),
        ],
      ),
    );
  }
}
