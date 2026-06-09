import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';

import '../../shared/theme/theme.dart';
import '../profile/user_cache_provider.dart';
import 'note_card.dart';
import 'pulse_models.dart';

class AgentActivityCard extends HookConsumerWidget {
  final AgentNoteGroup group;
  final Map<String, PulseReactionState> reactions;
  final VoidCallback? onReactionChanged;

  const AgentActivityCard({
    super.key,
    required this.group,
    required this.reactions,
    this.onReactionChanged,
  });

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final expanded = useState(group.notes.length == 1);
    final profile =
        ref.watch(userCacheProvider.select((cache) => cache[group.pubkey])) ??
        ref.read(userCacheProvider.notifier).get(group.pubkey);
    final name = profile?.label ?? _shortPubkey(group.pubkey);

    return Column(
      children: [
        InkWell(
          onTap: group.notes.length > 1
              ? () => expanded.value = !expanded.value
              : null,
          borderRadius: BorderRadius.circular(Radii.md),
          child: Padding(
            padding: const EdgeInsets.symmetric(vertical: Grid.twelve),
            child: Row(
              children: [
                Stack(
                  children: [
                    CircleAvatar(
                      radius: 18,
                      backgroundColor: context.colors.primaryContainer,
                      backgroundImage: profile?.avatarUrl != null
                          ? NetworkImage(profile!.avatarUrl!)
                          : null,
                      child: profile?.avatarUrl == null
                          ? const Icon(LucideIcons.bot, size: 18)
                          : null,
                    ),
                    Positioned(
                      right: 0,
                      bottom: 0,
                      child: Container(
                        width: 10,
                        height: 10,
                        decoration: BoxDecoration(
                          color: context.appColors.success,
                          shape: BoxShape.circle,
                          border: Border.all(
                            color: context.colors.surface,
                            width: 1.5,
                          ),
                        ),
                      ),
                    ),
                  ],
                ),
                const SizedBox(width: Grid.xs),
                Expanded(
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      Row(
                        children: [
                          Flexible(
                            child: Text(
                              name,
                              overflow: TextOverflow.ellipsis,
                              style: context.textTheme.labelLarge?.copyWith(
                                fontWeight: FontWeight.w700,
                              ),
                            ),
                          ),
                          const SizedBox(width: Grid.half),
                          Container(
                            padding: const EdgeInsets.symmetric(
                              horizontal: Grid.half,
                              vertical: 2,
                            ),
                            decoration: BoxDecoration(
                              color: context.colors.primary.withValues(
                                alpha: 0.12,
                              ),
                              borderRadius: BorderRadius.circular(Radii.sm),
                            ),
                            child: Text(
                              'BOT',
                              style: context.textTheme.labelSmall?.copyWith(
                                color: context.colors.primary,
                                fontWeight: FontWeight.w800,
                                fontSize: 10,
                              ),
                            ),
                          ),
                        ],
                      ),
                      Text(
                        '${group.notes.length} update${group.notes.length == 1 ? '' : 's'} · ${formatPulseRelativeTime(group.latestAt)}',
                        style: context.textTheme.labelSmall?.copyWith(
                          color: context.colors.onSurfaceVariant,
                        ),
                      ),
                    ],
                  ),
                ),
                if (group.notes.length > 1)
                  Icon(
                    expanded.value
                        ? LucideIcons.chevronUp
                        : LucideIcons.chevronDown,
                    size: 18,
                    color: context.colors.onSurfaceVariant,
                  ),
              ],
            ),
          ),
        ),
        if (expanded.value)
          Padding(
            padding: const EdgeInsets.only(left: Grid.xl),
            child: Column(
              children: [
                for (final note in group.notes) ...[
                  NoteCard(
                    note: note,
                    reaction:
                        reactions[note.id] ??
                        const PulseReactionState(
                          count: 0,
                          reactedByCurrentUser: false,
                        ),
                    isAgent: true,
                    onReactionChanged: onReactionChanged,
                  ),
                  if (note != group.notes.last)
                    Divider(
                      height: 1,
                      thickness: 1,
                      color: context.colors.outlineVariant.withValues(
                        alpha: 0.4,
                      ),
                    ),
                ],
              ],
            ),
          )
        else
          Padding(
            padding: const EdgeInsets.only(left: Grid.xl, bottom: Grid.xxs),
            child: Align(
              alignment: Alignment.centerLeft,
              child: Text(
                group.notes.first.content,
                maxLines: 2,
                overflow: TextOverflow.ellipsis,
                style: context.textTheme.bodyMedium,
              ),
            ),
          ),
      ],
    );
  }
}

String _shortPubkey(String pubkey) =>
    pubkey.length <= 8 ? pubkey : '${pubkey.substring(0, 8)}…';
