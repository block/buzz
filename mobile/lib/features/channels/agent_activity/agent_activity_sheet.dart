import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';

import '../../../shared/theme/theme.dart';
import '../../../shared/widgets/buzz_loading_indicator.dart';
import '../../profile/user_cache_provider.dart';
import '../date_formatters.dart';
import 'agent_activity_mode.dart';
import 'observer_models.dart';
import 'observer_subscription.dart';
import 'shared_activity_subscription.dart';
import 'shared_activity_summary.dart';
import 'transcript_item_widget.dart';

/// Selects the privileged owner transcript or member-safe shared stream.
class AgentActivitySheet extends StatelessWidget {
  final String channelId;
  final String agentPubkey;
  final String? ownerPubkey;
  final String? currentPubkey;
  final String channelType;
  final bool isCurrentMember;

  const AgentActivitySheet({
    super.key,
    required this.channelId,
    required this.agentPubkey,
    required this.ownerPubkey,
    required this.currentPubkey,
    required this.channelType,
    required this.isCurrentMember,
  });

  @override
  Widget build(BuildContext context) {
    final mode = selectAgentActivityMode(
      ownerPubkey: ownerPubkey?.toLowerCase(),
      myPubkey: currentPubkey?.toLowerCase(),
      channelType: channelType,
      isCurrentMember: isCurrentMember,
    );
    return switch (mode) {
      AgentActivityMode.owner => _OwnerAgentActivitySheet(
        channelId: channelId,
        agentPubkey: agentPubkey,
      ),
      AgentActivityMode.shared => _SharedAgentActivitySheet(
        channelId: channelId,
        agentPubkey: agentPubkey,
      ),
      AgentActivityMode.unavailable => const _UnavailableAgentActivitySheet(),
    };
  }
}

/// Full-screen modal bottom sheet showing the owner-only live transcript.
class _OwnerAgentActivitySheet extends HookConsumerWidget {
  final String channelId;
  final String agentPubkey;

  const _OwnerAgentActivitySheet({
    required this.channelId,
    required this.agentPubkey,
  });

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final observerState = ref.watch(
      observerSubscriptionProvider((
        channelId: channelId,
        agentPubkey: agentPubkey,
      )),
    );
    final transcript = observerState.transcript;
    final connection = observerState.connection;

    // Resolve bot name.
    final profile = ref.watch(
      userCacheProvider.select((cache) => cache[agentPubkey.toLowerCase()]),
    );
    final botName = profile?.label ?? shortPubkey(agentPubkey);

    // Auto-scroll to bottom on new items.
    final sheetControllerRef = useRef<ScrollController?>(null);
    final previousLength = useRef(0);

    useEffect(() {
      final sc = sheetControllerRef.value;
      if (transcript.length > previousLength.value &&
          sc != null &&
          sc.hasClients) {
        WidgetsBinding.instance.addPostFrameCallback((_) {
          if (sc.hasClients) {
            sc.animateTo(
              sc.position.maxScrollExtent,
              duration: const Duration(milliseconds: 150),
              curve: Curves.easeOut,
            );
          }
        });
      }
      previousLength.value = transcript.length;
      return null;
    }, [transcript.length]);

    // Preload the bot profile.
    useEffect(() {
      ref.read(userCacheProvider.notifier).preload([agentPubkey]);
      return null;
    }, [agentPubkey]);

    return DraggableScrollableSheet(
      initialChildSize: 0.9,
      minChildSize: 0.5,
      maxChildSize: 0.95,
      expand: false,
      builder: (context, sheetScrollController) {
        sheetControllerRef.value = sheetScrollController;
        final bottomPadding =
            MediaQuery.viewPaddingOf(context).bottom + Grid.sm;
        return Column(
          children: [
            Padding(
              padding: const EdgeInsets.symmetric(horizontal: Grid.gutter),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Row(
                    children: [
                      Icon(
                        LucideIcons.bot,
                        size: 18,
                        color: context.colors.onSurface,
                      ),
                      const SizedBox(width: Grid.xxs),
                      Expanded(
                        child: Text(
                          botName,
                          style: context.textTheme.titleMedium?.copyWith(
                            fontWeight: FontWeight.w600,
                          ),
                          overflow: TextOverflow.ellipsis,
                        ),
                      ),
                      _ConnectionBadge(connection: connection),
                    ],
                  ),
                  const SizedBox(height: Grid.half),
                  Text(
                    'Showing live activity from this point.',
                    style: context.textTheme.bodySmall?.copyWith(
                      color: context.colors.onSurfaceVariant,
                    ),
                  ),
                  const SizedBox(height: Grid.xxs),
                  Divider(color: context.colors.outlineVariant),
                ],
              ),
            ),
            // Transcript list
            Expanded(
              child: transcript.isEmpty
                  ? Padding(
                      padding: EdgeInsets.only(bottom: bottomPadding),
                      child: _EmptyState(
                        connection: connection,
                        errorMessage: observerState.errorMessage,
                      ),
                    )
                  : ListView.builder(
                      controller: sheetScrollController,
                      padding: EdgeInsets.fromLTRB(
                        Grid.gutter,
                        Grid.xxs,
                        Grid.gutter,
                        bottomPadding,
                      ),
                      itemCount: transcript.length,
                      itemBuilder: (context, index) {
                        return TranscriptItemWidget(item: transcript[index]);
                      },
                    ),
            ),
          ],
        );
      },
    );
  }
}

class _SharedAgentActivitySheet extends HookConsumerWidget {
  final String channelId;
  final String agentPubkey;

  const _SharedAgentActivitySheet({
    required this.channelId,
    required this.agentPubkey,
  });

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final activityState = ref.watch(
      sharedActivitySubscriptionProvider((
        channelId: channelId,
        agentPubkey: agentPubkey.toLowerCase(),
      )),
    );
    final profile = ref.watch(
      userCacheProvider.select((cache) => cache[agentPubkey.toLowerCase()]),
    );
    final botName = profile?.label ?? shortPubkey(agentPubkey);

    useEffect(() {
      ref.read(userCacheProvider.notifier).preload([agentPubkey]);
      return null;
    }, [agentPubkey]);

    return DraggableScrollableSheet(
      initialChildSize: 0.9,
      minChildSize: 0.5,
      maxChildSize: 0.95,
      expand: false,
      builder: (context, sheetScrollController) {
        final bottomPadding =
            MediaQuery.viewPaddingOf(context).bottom + Grid.sm;
        return Column(
          children: [
            Padding(
              padding: const EdgeInsets.symmetric(horizontal: Grid.gutter),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Row(
                    children: [
                      Icon(
                        LucideIcons.bot,
                        size: 18,
                        color: context.colors.onSurface,
                      ),
                      const SizedBox(width: Grid.xxs),
                      Expanded(
                        child: Text(
                          botName,
                          style: context.textTheme.titleMedium?.copyWith(
                            fontWeight: FontWeight.w600,
                          ),
                          overflow: TextOverflow.ellipsis,
                        ),
                      ),
                      _SharedConnectionBadge(
                        connection: activityState.connection,
                      ),
                    ],
                  ),
                  const SizedBox(height: Grid.half),
                  Text(
                    'Showing privacy-safe live activity from this point.',
                    style: context.textTheme.bodySmall?.copyWith(
                      color: context.colors.onSurfaceVariant,
                    ),
                  ),
                  const SizedBox(height: Grid.xxs),
                  Divider(color: context.colors.outlineVariant),
                ],
              ),
            ),
            Expanded(
              child: activityState.activities.isEmpty
                  ? _SharedEmptyState(state: activityState)
                  : SharedActivitySummary(
                      activities: activityState.activities,
                      controller: sheetScrollController,
                      padding: EdgeInsets.fromLTRB(
                        Grid.gutter,
                        Grid.xxs,
                        Grid.gutter,
                        bottomPadding,
                      ),
                    ),
            ),
          ],
        );
      },
    );
  }
}

class _UnavailableAgentActivitySheet extends StatelessWidget {
  const _UnavailableAgentActivitySheet();

  @override
  Widget build(BuildContext context) => const SizedBox(
    height: 320,
    child: Center(
      child: Padding(
        padding: EdgeInsets.all(Grid.gutter),
        child: Text(
          'Live activity is available to current channel members.',
          textAlign: TextAlign.center,
        ),
      ),
    ),
  );
}

class _SharedEmptyState extends StatelessWidget {
  final SharedActivityState state;

  const _SharedEmptyState({required this.state});

  @override
  Widget build(BuildContext context) {
    final failed =
        state.connection == SharedActivityConnectionState.closed ||
        state.connection == SharedActivityConnectionState.error;
    if (failed) {
      return Center(
        child: Padding(
          padding: const EdgeInsets.all(Grid.gutter),
          child: Text(
            state.errorMessage ?? 'Shared activity is unavailable.',
            style: context.textTheme.bodySmall?.copyWith(
              color: context.colors.onSurfaceVariant,
            ),
            textAlign: TextAlign.center,
          ),
        ),
      );
    }
    return Center(
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          BuzzLoadingIndicator(
            size: 28,
            color: context.colors.onSurfaceVariant,
            semanticLabel: 'Waiting for privacy-safe agent activity',
          ),
          const SizedBox(height: Grid.xxs),
          Text(
            'Waiting for activity\u2026',
            style: context.textTheme.bodySmall?.copyWith(
              color: context.colors.onSurfaceVariant,
            ),
          ),
        ],
      ),
    );
  }
}

class _SharedConnectionBadge extends StatelessWidget {
  final SharedActivityConnectionState connection;

  const _SharedConnectionBadge({required this.connection});

  @override
  Widget build(BuildContext context) {
    final (color, label) = switch (connection) {
      SharedActivityConnectionState.connecting => (
        context.appColors.warning,
        'Connecting',
      ),
      SharedActivityConnectionState.live => (context.appColors.success, 'Live'),
      SharedActivityConnectionState.closed => (
        context.colors.onSurfaceVariant,
        'Closed',
      ),
      SharedActivityConnectionState.error => (context.colors.error, 'Error'),
    };
    return _ActivityBadge(color: color, label: label);
  }
}

class _EmptyState extends StatelessWidget {
  final ObserverConnectionState connection;
  final String? errorMessage;

  const _EmptyState({required this.connection, this.errorMessage});

  @override
  Widget build(BuildContext context) {
    if (connection == ObserverConnectionState.error) {
      return Center(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            Icon(LucideIcons.circleX, size: 24, color: context.colors.error),
            const SizedBox(height: Grid.xxs),
            Text(
              'Error: ${errorMessage ?? 'Unknown error'}',
              style: context.textTheme.bodySmall?.copyWith(
                color: context.colors.error,
              ),
              textAlign: TextAlign.center,
            ),
          ],
        ),
      );
    }

    if (connection == ObserverConnectionState.idle) {
      return Center(
        child: Text(
          'Not connected',
          style: context.textTheme.bodySmall?.copyWith(
            color: context.colors.onSurfaceVariant,
          ),
        ),
      );
    }

    // connecting or open — show spinner
    return Center(
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          BuzzLoadingIndicator(
            size: 28,
            color: context.colors.onSurfaceVariant,
            semanticLabel: 'Waiting for agent activity',
          ),
          const SizedBox(height: Grid.xxs),
          Text(
            'Waiting for activity\u2026',
            style: context.textTheme.bodySmall?.copyWith(
              color: context.colors.onSurfaceVariant,
            ),
          ),
        ],
      ),
    );
  }
}

class _ConnectionBadge extends StatelessWidget {
  final ObserverConnectionState connection;

  const _ConnectionBadge({required this.connection});

  @override
  Widget build(BuildContext context) {
    final (color, label) = switch (connection) {
      ObserverConnectionState.idle => (context.colors.onSurfaceVariant, 'Idle'),
      ObserverConnectionState.connecting => (
        context.appColors.warning,
        'Connecting',
      ),
      ObserverConnectionState.open => (context.appColors.success, 'Live'),
      ObserverConnectionState.error => (context.colors.error, 'Error'),
    };

    return _ActivityBadge(color: color, label: label);
  }
}

class _ActivityBadge extends StatelessWidget {
  final Color color;
  final String label;

  const _ActivityBadge({required this.color, required this.label});

  @override
  Widget build(BuildContext context) {
    return Container(
      padding: const EdgeInsets.symmetric(
        horizontal: Grid.xxs,
        vertical: Grid.quarter,
      ),
      decoration: BoxDecoration(
        color: color.withValues(alpha: 0.12),
        borderRadius: BorderRadius.circular(12),
      ),
      child: Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          Container(
            width: 6,
            height: 6,
            decoration: BoxDecoration(color: color, shape: BoxShape.circle),
          ),
          const SizedBox(width: Grid.half),
          Text(
            label,
            style: context.textTheme.labelSmall?.copyWith(
              color: color,
              fontWeight: FontWeight.w600,
            ),
          ),
        ],
      ),
    );
  }
}
