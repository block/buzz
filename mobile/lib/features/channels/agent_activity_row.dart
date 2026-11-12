import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/mentions/agent_identity_provider.dart';
import '../../shared/profile/user_cache_provider.dart';
import '../../shared/theme/buzz_icons.dart';
import '../../shared/theme/theme.dart';
import '../../shared/widgets/avatar_image.dart';
import 'agent_activity_runs.dart';
import 'channel_identity_names_provider.dart';
import 'message_content.dart';
import 'timeline_message.dart';

/// The activity runs of one displayed timeline level, and which are open.
class ActivityTimeline {
  final Map<String, ActivityRun> runs;

  /// IDs of messages whose runs are open, so a run stays open as older or
  /// newer steps join it.
  final ValueNotifier<Set<String>> expanded;
  final String channelId;

  const ActivityTimeline(this.runs, this.expanded, this.channelId);

  ActivityRun? operator [](String id) => runs[id];

  bool isExpanded(String id) =>
      runs[id]?.rows.any((row) => expanded.value.contains(row.id)) ?? false;

  /// [row] as an item of [message]'s run, if it has one.
  Widget item(TimelineMessage message, Widget row) {
    final run = runs[message.id];
    if (run == null) return row;
    final open = isExpanded(message.id);
    return AgentActivityRow(
      run: run,
      message: message,
      expanded: open,
      channelId: channelId,
      onToggle: () => expanded.value = open
          ? expanded.value.difference({for (final r in run.rows) r.id})
          : {...expanded.value, run.first.id},
      child: row,
    );
  }
}

/// The [ActivityTimeline] of [rows] in [channelId], recomputed when [keys]
/// change. See [activityRuns] for [loaded] and [rootId].
ActivityTimeline useActivityTimeline(
  WidgetRef ref,
  String channelId,
  List<TimelineMessage> Function() rows, {
  required List<TimelineMessage> loaded,
  String? rootId,
  required List<Object?> keys,
}) {
  final agents = ref.watch(agentMentionPubkeysProvider(channelId));
  final runs = useMemoized(
    () => activityRuns(rows(), loaded: loaded, rootId: rootId, agents: agents),
    [...keys, loaded, rootId, agents],
  );
  return ActivityTimeline(runs, useState(<String>{}), channelId);
}

/// One timeline item of an activity [run], so list indexes, deep links and
/// read tracking keep addressing each message. Collapsed, the run's first
/// item shows its preview and the others are empty. Expanded, each item is
/// its ordinary [child] beside a rule. The newest step carries the Collapse
/// control: the timeline keeps newer messages in place, so it opens where the
/// preview was, under the finger that expanded it.
class AgentActivityRow extends StatelessWidget {
  final ActivityRun run;
  final TimelineMessage message;
  final bool expanded;
  final VoidCallback onToggle;
  final String channelId;
  final Widget child;

  const AgentActivityRow({
    super.key,
    required this.run,
    required this.message,
    required this.expanded,
    required this.onToggle,
    required this.channelId,
    required this.child,
  });

  @override
  Widget build(BuildContext context) {
    if (!expanded) {
      return message.id == run.first.id
          ? _ActivityPreview(run: run, channelId: channelId, onTap: onToggle)
          : const SizedBox.shrink();
    }
    return DecoratedBox(
      decoration: BoxDecoration(
        border: Border(
          left: BorderSide(color: context.colors.outlineVariant, width: 2),
        ),
      ),
      child: Padding(
        padding: const EdgeInsets.only(left: Grid.xxs),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            child,
            if (message.id == run.latest.id)
              Align(
                alignment: Alignment.centerLeft,
                child: Semantics(
                  expanded: true,
                  child: TextButton.icon(
                    key: ValueKey('activity-collapse-${run.first.id}'),
                    onPressed: onToggle,
                    style: TextButton.styleFrom(
                      minimumSize: const Size(0, Grid.xl),
                      foregroundColor: context.colors.onSurfaceVariant,
                    ),
                    icon: const Icon(BuzzIcons.chevronUp, size: 16),
                    label: const Text('Collapse'),
                  ),
                ),
              ),
          ],
        ),
      ),
    );
  }
}

class _ActivityPreview extends ConsumerWidget {
  final ActivityRun run;
  final String channelId;
  final VoidCallback onTap;

  const _ActivityPreview({
    required this.run,
    required this.channelId,
    required this.onTap,
  });

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final primary = run.first.pubkey.toLowerCase();
    final latest = run.latest;
    final speaker = latest.pubkey.toLowerCase();
    final profile =
        ref.watch(userCacheProvider.select((cache) => cache[primary])) ??
        ref.read(userCacheProvider.notifier).get(primary);
    final name = watchChannelIdentityLabel(ref, channelId, primary);
    final speakerName = watchChannelIdentityLabel(ref, channelId, speaker);
    final isAgent =
        ref.watch(agentMentionPubkeysProvider(channelId)).contains(primary) ||
        profile?.ownerPubkey != null;
    final textStyle = messageBodyTextStyle.copyWith(
      color: context.colors.onSurfaceVariant,
    );
    return Padding(
      padding: const EdgeInsets.only(top: Grid.xs),
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.end,
        children: [
          AvatarImage(
            imageUrl: profile?.avatarUrl,
            radius: messageAvatarSize / 2,
            backgroundColor: context.controlSurface,
            isAgent: isAgent,
            fallback: Text(
              profile?.initial ??
                  (primary.isEmpty ? '?' : primary[0].toUpperCase()),
              style: context.textTheme.labelMedium?.copyWith(
                color: context.colors.onPrimaryContainer,
                fontWeight: FontWeight.w600,
              ),
            ),
          ),
          const SizedBox(width: messageAvatarContentGap),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Padding(
                  padding: const EdgeInsets.only(
                    left: Grid.xs,
                    bottom: Grid.quarter,
                  ),
                  child: Text(
                    name,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: messageUsernameTextStyle.copyWith(
                      fontWeight: FontWeight.w500,
                      fontSize: 11,
                      height: 17 / 11,
                      color: context.tertiaryTextColor,
                    ),
                  ),
                ),
                Semantics(
                  button: true,
                  expanded: false,
                  label: 'Show activity history',
                  child: Material(
                    color: context.controlSurface,
                    borderRadius: BorderRadius.circular(Radii.container),
                    child: InkWell(
                      key: ValueKey('activity-preview-${run.first.id}'),
                      borderRadius: BorderRadius.circular(Radii.container),
                      onTap: onTap,
                      child: ConstrainedBox(
                        constraints: const BoxConstraints(minHeight: Grid.xl),
                        child: Padding(
                          padding: const EdgeInsets.symmetric(
                            horizontal: Grid.xs,
                            vertical: Grid.xxs,
                          ),
                          child: Row(
                            children: [
                              if (run.participants.length > 1)
                                Text('$speakerName: ', style: textStyle),
                              Expanded(
                                child: MessagePreview(
                                  content: latest.content,
                                  tags: latest.tags,
                                  maxLines: 1,
                                  baseStyle: textStyle,
                                ),
                              ),
                              const SizedBox(width: Grid.xxs),
                              Icon(
                                BuzzIcons.chevronDown,
                                size: 16,
                                color: context.colors.onSurfaceVariant,
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
        ],
      ),
    );
  }
}
