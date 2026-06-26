import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';

import '../../shared/theme/theme.dart';
import '../channels/message_content.dart';
import '../profile/user_cache_provider.dart';
import '../profile/user_profile_sheet.dart';
import '../profile/user_profile.dart';
import 'forum_models.dart';

/// Card displaying a forum post preview in the posts list.
///
/// Long-press opens an action sheet (copy, delete) matching the stream
/// message pattern from channel_detail_page.dart.
class ForumPostCard extends ConsumerWidget {
  final ForumPost post;
  final String? currentPubkey;
  final VoidCallback onTap;
  final void Function(String eventId)? onDelete;

  const ForumPostCard({
    super.key,
    required this.post,
    required this.currentPubkey,
    required this.onTap,
    this.onDelete,
  });

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final pk = post.pubkey.toLowerCase();
    final profile =
        ref.watch(userCacheProvider.select((cache) => cache[pk])) ??
        ref.read(userCacheProvider.notifier).get(pk);
    final displayName = profile?.label ?? _shortPubkey(post.pubkey);
    final mentionNames = ref.watch(
      userCacheProvider.select(
        (cache) => _buildMentionNames(post.mentionPubkeys, cache),
      ),
    );
    final preview = post.content.length > 200
        ? '${post.content.substring(0, 200)}...'
        : post.content;
    final summary = post.threadSummary;

    return GestureDetector(
      onTap: onTap,
      onLongPress: () => _showActions(context),
      child: Container(
        width: double.infinity,
        padding: const EdgeInsets.all(Grid.twelve),
        decoration: BoxDecoration(
          color: context.colors.surfaceContainerHighest,
          borderRadius: BorderRadius.circular(Radii.lg),
          border: Border.all(
            color: context.colors.outlineVariant.withValues(alpha: 0.5),
          ),
        ),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            // Author row
            Row(
              children: [
                GestureDetector(
                  behavior: HitTestBehavior.opaque,
                  onTap: () => showUserProfileSheet(context, post.pubkey),
                  child: _PostAvatar(profile: profile, pubkey: post.pubkey),
                ),
                const SizedBox(width: Grid.xxs),
                Expanded(
                  child: GestureDetector(
                    behavior: HitTestBehavior.opaque,
                    onTap: () => showUserProfileSheet(context, post.pubkey),
                    child: Text(
                      displayName,
                      style: context.textTheme.labelMedium?.copyWith(
                        fontWeight: FontWeight.w600,
                      ),
                      overflow: TextOverflow.ellipsis,
                    ),
                  ),
                ),
                Text(
                  formatRelativeTime(post.createdAt),
                  style: context.textTheme.labelSmall?.copyWith(
                    color: context.colors.onSurfaceVariant,
                  ),
                ),
                const SizedBox(width: Grid.half),
                SizedBox(
                  width: 24,
                  height: 24,
                  child: IconButton(
                    onPressed: () => _showActions(context),
                    icon: Icon(
                      LucideIcons.ellipsis,
                      size: 16,
                      color: context.colors.onSurfaceVariant,
                    ),
                    padding: EdgeInsets.zero,
                    visualDensity: VisualDensity.compact,
                  ),
                ),
              ],
            ),
            const SizedBox(height: Grid.xxs),

            ShaderMask(
              shaderCallback: (bounds) => const LinearGradient(
                begin: Alignment.topCenter,
                end: Alignment.bottomCenter,
                colors: [Colors.white, Colors.white, Colors.transparent],
                stops: [0.0, 0.75, 1.0],
              ).createShader(bounds),
              blendMode: BlendMode.dstIn,
              child: ConstrainedBox(
                constraints: const BoxConstraints(maxHeight: 120),
                child: IgnorePointer(
                  child: MessageContent(
                    content: preview,
                    mentionNames: mentionNames,
                    tags: post.tags,
                  ),
                ),
              ),
            ),

            // Thread summary
            if (summary != null && summary.replyCount > 0) ...[
              const SizedBox(height: Grid.xxs),
              Row(
                children: [
                  Icon(
                    LucideIcons.messageSquare,
                    size: 14,
                    color: context.colors.onSurfaceVariant,
                  ),
                  const SizedBox(width: Grid.half),
                  Text(
                    '${summary.replyCount} ${summary.replyCount == 1 ? 'reply' : 'replies'}',
                    style: context.textTheme.labelSmall?.copyWith(
                      color: context.colors.onSurfaceVariant,
                    ),
                  ),
                  if (summary.lastReplyAt != null) ...[
                    const SizedBox(width: Grid.half),
                    Text(
                      '\u00b7',
                      style: context.textTheme.labelSmall?.copyWith(
                        color: context.colors.onSurfaceVariant.withValues(
                          alpha: 0.5,
                        ),
                      ),
                    ),
                    const SizedBox(width: Grid.half),
                    Text(
                      'last ${formatRelativeTime(summary.lastReplyAt!)}',
                      style: context.textTheme.labelSmall?.copyWith(
                        color: context.colors.onSurfaceVariant,
                      ),
                    ),
                  ],
                ],
              ),
            ],
          ],
        ),
      ),
    );
  }

  void _showActions(BuildContext context) {
    final isOwn =
        currentPubkey != null &&
        post.pubkey.toLowerCase() == currentPubkey!.toLowerCase();

    showModalBottomSheet<void>(
      context: context,
      showDragHandle: true,
      builder: (sheetContext) => SafeArea(
        child: Padding(
          padding: const EdgeInsets.fromLTRB(Grid.xs, 0, Grid.xs, Grid.xs),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              ListTile(
                leading: const Icon(LucideIcons.copy),
                title: const Text('Copy text'),
                onTap: () {
                  Navigator.of(sheetContext).pop();
                  Clipboard.setData(ClipboardData(text: post.content));
                },
              ),
              if (isOwn && onDelete != null)
                ListTile(
                  leading: Icon(
                    LucideIcons.trash2,
                    color: Theme.of(sheetContext).colorScheme.error,
                  ),
                  title: Text(
                    'Delete post',
                    style: TextStyle(
                      color: Theme.of(sheetContext).colorScheme.error,
                    ),
                  ),
                  onTap: () {
                    Navigator.of(sheetContext).pop();
                    _confirmDelete(context);
                  },
                ),
            ],
          ),
        ),
      ),
    );
  }

  void _confirmDelete(BuildContext context) {
    showDialog<void>(
      context: context,
      builder: (dialogContext) => AlertDialog(
        title: const Text('Delete post'),
        content: const Text('This cannot be undone.'),
        actions: [
          TextButton(
            onPressed: () => Navigator.of(dialogContext).pop(),
            child: const Text('Cancel'),
          ),
          FilledButton(
            onPressed: () {
              Navigator.of(dialogContext).pop();
              onDelete?.call(post.eventId);
            },
            style: FilledButton.styleFrom(
              backgroundColor: Theme.of(dialogContext).colorScheme.error,
            ),
            child: const Text('Delete'),
          ),
        ],
      ),
    );
  }
}

class _PostAvatar extends StatelessWidget {
  final UserProfile? profile;
  final String pubkey;

  const _PostAvatar({required this.profile, required this.pubkey});

  @override
  Widget build(BuildContext context) {
    final initial =
        profile?.initial ?? (pubkey.isNotEmpty ? pubkey[0].toUpperCase() : '?');
    final avatarUrl = profile?.avatarUrl;

    return CircleAvatar(
      radius: 14,
      backgroundColor: context.colors.primaryContainer,
      backgroundImage: avatarUrl != null ? NetworkImage(avatarUrl) : null,
      child: avatarUrl == null
          ? Text(
              initial,
              style: context.textTheme.labelSmall?.copyWith(
                color: context.colors.onPrimaryContainer,
                fontWeight: FontWeight.w600,
              ),
            )
          : null,
    );
  }
}

String _shortPubkey(String pubkey) {
  if (pubkey.length > 12) return '${pubkey.substring(0, 8)}\u2026';
  return pubkey;
}

Map<String, String> _buildMentionNames(
  List<String> mentionPubkeys,
  Map<String, UserProfile> userCache,
) {
  final names = <String, String>{};
  for (final pk in mentionPubkeys) {
    final p = userCache[pk.toLowerCase()];
    if (p?.displayName != null) {
      names[pk.toLowerCase()] = p!.displayName!;
    }
  }
  return names;
}
