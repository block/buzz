import 'package:flutter/material.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/profile/user_cache_provider.dart';
import '../../shared/theme/theme.dart';
import 'channel_identity_names_provider.dart';
import 'conversation_activity_state.dart';
import 'small_avatar.dart';
import 'timeline_message.dart';
import 'typing_dots.dart';
import 'working_now_sheet.dart';

/// Desktop's conversation activity above the composer: the user's agents
/// working here, which open the "Working now" list, and everyone else typing.
class ConversationActivity extends ConsumerWidget {
  const ConversationActivity({
    super.key,
    required this.scope,
    this.messages = const [],
    this.onOpenThread,
  });

  final ActivityScope scope;

  /// Loaded messages, to name and open the threads agents work in.
  final List<TimelineMessage> messages;
  final void Function(TimelineMessage root)? onOpenThread;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final activity = watchConversationActivity(ref, scope);
    if (activity == null) return const SizedBox.shrink();
    final (:agents, :typers) = activity;
    if (agents.isEmpty && typers.isEmpty) return const SizedBox.shrink();
    final labels = watchChannelIdentityLabels(ref, scope.channelId, {
      ...agents.keys,
      ...typers,
    });
    String name(String pubkey) => labels[pubkey] ?? pubkey.substring(0, 10);
    // Desktop's strings, in TypingIndicator.
    final working = agents.isEmpty
        ? ''
        : '${agents.keys.map(name).join(', ')}'
              '${agents.length == 1 ? ' is working' : ' are working'}';
    final typing = typers.isEmpty
        ? ''
        : '${typers.take(2).map(name).join(', ')}'
              '${typers.length > 2 ? ' and ${typers.length - 2} others' : ''}'
              '${typers.length == 1 ? ' is typing' : ' are typing'}';
    final status = [working, typing].where((text) => text.isNotEmpty);
    return Padding(
      key: const ValueKey('conversation-activity'),
      padding: const EdgeInsets.fromLTRB(Grid.twelve, 0, Grid.twelve, Grid.xxs),
      child: Row(
        children: [
          // The status speaks for both sides, as desktop's live region does.
          Expanded(
            child: Semantics(
              liveRegion: true,
              label: status.join('. '),
              child: ExcludeSemantics(
                child: Align(
                  alignment: AlignmentDirectional.centerStart,
                  child: typing.isEmpty ? null : _Typers(typing),
                ),
              ),
            ),
          ),
          if (agents.isNotEmpty)
            _WorkingAgents(
              agents: agents.keys.toList(),
              label: agents.length == 1
                  ? 'Activity: ${name(agents.keys.single)} working'
                  : 'Activity: ${agents.length} agents working',
              onTap: () => showWorkingNowSheet(
                context,
                scope: scope,
                messages: messages,
                onOpenThread: onOpenThread,
              ),
            ),
        ],
      ),
    );
  }
}

class _Typers extends StatelessWidget {
  const _Typers(this.text);
  final String text;

  @override
  Widget build(BuildContext context) => FractionallySizedBox(
    // Desktop keeps the typing text to half the row.
    widthFactor: 0.5,
    alignment: AlignmentDirectional.centerStart,
    child: Align(
      alignment: AlignmentDirectional.centerStart,
      child: Container(
        padding: const EdgeInsets.symmetric(
          horizontal: Grid.xxs,
          vertical: Grid.half,
        ),
        decoration: BoxDecoration(
          color: context.colors.surfaceContainerHighest,
          borderRadius: BorderRadius.circular(Radii.container),
        ),
        child: Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            const TypingDots(dotSize: 5),
            const SizedBox(width: Grid.xxs),
            Flexible(
              child: Text(
                text,
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
                style: context.textTheme.labelSmall?.copyWith(
                  color: context.colors.onSurfaceVariant,
                ),
              ),
            ),
          ],
        ),
      ),
    ),
  );
}

class _WorkingAgents extends ConsumerWidget {
  const _WorkingAgents({
    required this.agents,
    required this.label,
    required this.onTap,
  });
  final List<String> agents;
  final String label;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final userCache = ref.watch(userCacheProvider);
    // Desktop shows at most four avatars, then a count.
    final shown = agents.take(4).toList();
    // Desktop's 24px tiles, each overlapping the next by 4px.
    const tile = 24.0, step = tile - Grid.half;
    return Semantics(
      button: true,
      label: label,
      onTap: onTap,
      excludeSemantics: true,
      child: InkWell(
        key: const ValueKey('conversation-activity-agents'),
        onTap: onTap,
        borderRadius: BorderRadius.circular(Radii.container),
        child: ConstrainedBox(
          constraints: const BoxConstraints(minHeight: 44, minWidth: 44),
          child: Padding(
            padding: const EdgeInsets.symmetric(horizontal: Grid.half),
            child: Row(
              mainAxisSize: MainAxisSize.min,
              children: [
                SizedBox(
                  width: tile + step * (shown.length - 1),
                  height: tile,
                  child: Stack(
                    children: [
                      for (final (i, pubkey) in shown.indexed)
                        PositionedDirectional(
                          start: step * i,
                          child: Container(
                            padding: const EdgeInsets.all(Grid.quarter),
                            decoration: BoxDecoration(
                              color: context.colors.surfaceContainerHighest,
                              borderRadius: BorderRadius.circular(
                                Radii.container,
                              ),
                              boxShadow: [
                                BoxShadow(
                                  color: Colors.black.withValues(alpha: 0.08),
                                  blurRadius: 2,
                                  offset: const Offset(0, 1),
                                ),
                              ],
                            ),
                            child: SmallAvatar(
                              pubkey: pubkey,
                              userCache: userCache,
                              size: tile - 2 * Grid.quarter,
                              agent: true,
                            ),
                          ),
                        ),
                    ],
                  ),
                ),
                if (agents.length > shown.length)
                  Padding(
                    padding: const EdgeInsetsDirectional.only(start: Grid.xxs),
                    child: Text(
                      '+${agents.length - shown.length}',
                      style: context.textTheme.labelSmall?.copyWith(
                        color: context.colors.onSurfaceVariant,
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
