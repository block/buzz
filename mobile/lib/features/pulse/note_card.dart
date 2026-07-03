import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';
import 'package:nostr/nostr.dart' as nostr;

import '../../shared/clipboard_utils.dart';
import '../../shared/theme/theme.dart';
import '../channels/channel_detail_page.dart';
import '../channels/channel_management_provider.dart';
import '../channels/message_content.dart';
import '../profile/user_cache_provider.dart';
import '../profile/user_profile_sheet.dart';
import 'compose_note_page.dart';
import 'pulse_actions.dart';
import 'pulse_models.dart';

class NoteCard extends HookConsumerWidget {
  final UserNote note;
  final PulseReactionState reaction;
  final bool isAgent;
  final bool isFollowing;
  final bool canFollow;
  final VoidCallback? onReactionChanged;
  final ValueChanged<String>? onFollowChanged;

  const NoteCard({
    super.key,
    required this.note,
    required this.reaction,
    this.isAgent = false,
    this.isFollowing = false,
    this.canFollow = false,
    this.onReactionChanged,
    this.onFollowChanged,
  });

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final pendingUpvote = useState<bool?>(null);
    final pubkey = note.pubkey.toLowerCase();
    final profile =
        ref.watch(userCacheProvider.select((cache) => cache[pubkey])) ??
        ref.read(userCacheProvider.notifier).get(pubkey);
    final displayName = profile?.label ?? _shortPubkey(pubkey);
    final effectiveUpvoted =
        pendingUpvote.value ?? reaction.reactedByCurrentUser;
    final effectiveCount = _effectiveCount(reaction, pendingUpvote.value);

    // Clear the optimistic flag only once the refetched server state matches
    // it, so the count never flickers back through the stale value mid-refetch.
    useEffect(() {
      if (pendingUpvote.value != null &&
          reaction.reactedByCurrentUser == pendingUpvote.value) {
        pendingUpvote.value = null;
      }
      return null;
    }, [reaction.reactedByCurrentUser]);

    return Padding(
      padding: const EdgeInsets.symmetric(vertical: Grid.twelve),
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          GestureDetector(
            onTap: () => showUserProfileSheet(context, note.pubkey),
            child: CircleAvatar(
              radius: 18,
              backgroundColor: context.colors.primaryContainer,
              backgroundImage: profile?.avatarUrl != null
                  ? NetworkImage(profile!.avatarUrl!)
                  : null,
              child: profile?.avatarUrl == null
                  ? Text(
                      (profile?.initial ?? displayName[0]).toUpperCase(),
                      style: context.textTheme.labelMedium?.copyWith(
                        color: context.colors.onPrimaryContainer,
                      ),
                    )
                  : null,
            ),
          ),
          const SizedBox(width: Grid.xs),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Row(
                  children: [
                    Expanded(
                      child: GestureDetector(
                        onTap: () => showUserProfileSheet(context, note.pubkey),
                        child: Row(
                          children: [
                            Flexible(
                              child: Text(
                                displayName,
                                style: context.textTheme.labelMedium?.copyWith(
                                  fontWeight: FontWeight.w700,
                                ),
                                overflow: TextOverflow.ellipsis,
                              ),
                            ),
                            if (isAgent) ...[
                              const SizedBox(width: Grid.half),
                              Icon(
                                LucideIcons.bot,
                                size: 13,
                                color: context.colors.primary,
                              ),
                            ],
                          ],
                        ),
                      ),
                    ),
                    Text(
                      formatPulseRelativeTime(note.createdAt),
                      style: context.textTheme.labelSmall?.copyWith(
                        color: context.colors.onSurfaceVariant,
                      ),
                    ),
                    if (canFollow) ...[
                      const SizedBox(width: Grid.half),
                      _FollowButton(
                        isFollowing: isFollowing,
                        onPressed: () async {
                          if (isFollowing) {
                            await unfollowUser(ref, note.pubkey);
                          } else {
                            await followUser(ref, note.pubkey);
                          }
                          onFollowChanged?.call(note.pubkey);
                        },
                      ),
                    ],
                  ],
                ),
                if (note.replyParentId != null) ...[
                  const SizedBox(height: 2),
                  Text(
                    'Replying to ${_shortPubkey(note.replyParentAuthor ?? note.replyParentId!)}',
                    style: context.textTheme.labelSmall?.copyWith(
                      color: context.colors.onSurfaceVariant,
                    ),
                  ),
                ],
                const SizedBox(height: Grid.half),
                MessageContent(content: note.content, tags: note.tags),
                const SizedBox(height: Grid.xxs),
                Row(
                  children: [
                    _ActionButton(
                      icon: effectiveUpvoted
                          ? Icons.favorite
                          : Icons.favorite_border,
                      label: effectiveCount > 0 ? '$effectiveCount' : null,
                      color: effectiveUpvoted ? Colors.redAccent : null,
                      onTap: () async {
                        final next = !effectiveUpvoted;
                        pendingUpvote.value = next;
                        try {
                          await toggleNoteUpvote(
                            ref,
                            noteId: note.id,
                            isUpvoted: reaction.reactedByCurrentUser,
                            reactionEventId: reaction.currentUserReactionId,
                          );
                          onReactionChanged?.call();
                        } catch (_) {
                          // Revert the optimistic state on failure.
                          pendingUpvote.value = null;
                        }
                      },
                    ),
                    _ActionButton(
                      icon: LucideIcons.messageCircle,
                      onTap: () => Navigator.of(context).push(
                        MaterialPageRoute<void>(
                          builder: (_) => ComposeNotePage(replyTo: note),
                        ),
                      ),
                    ),
                    _ActionButton(
                      icon: LucideIcons.share,
                      onTap: () async {
                        await copyToClipboard(
                          context,
                          _shareUri(note),
                          message: 'Copied note URI',
                        );
                      },
                    ),
                    _ActionButton(
                      icon: LucideIcons.mail,
                      onTap: () async {
                        final channel = await ref
                            .read(channelActionsProvider)
                            .openDm(pubkeys: [note.pubkey]);
                        if (!context.mounted) return;
                        Navigator.of(context).push(
                          MaterialPageRoute<void>(
                            builder: (_) => ChannelDetailPage(channel: channel),
                          ),
                        );
                      },
                    ),
                  ],
                ),
              ],
            ),
          ),
        ],
      ),
    );
  }

  int _effectiveCount(PulseReactionState reaction, bool? pending) {
    if (pending == null || pending == reaction.reactedByCurrentUser) {
      return reaction.count;
    }
    return (reaction.count + (pending ? 1 : -1)).clamp(0, 1 << 31);
  }
}

class _FollowButton extends StatelessWidget {
  final bool isFollowing;
  final VoidCallback onPressed;

  const _FollowButton({required this.isFollowing, required this.onPressed});

  @override
  Widget build(BuildContext context) {
    return InkWell(
      onTap: onPressed,
      borderRadius: BorderRadius.circular(Radii.md),
      child: Padding(
        padding: const EdgeInsets.symmetric(horizontal: Grid.half, vertical: 2),
        child: Text(
          isFollowing ? 'Following' : 'Follow',
          style: context.textTheme.labelSmall?.copyWith(
            color: context.colors.primary,
            fontWeight: FontWeight.w700,
          ),
        ),
      ),
    );
  }
}

class _ActionButton extends StatelessWidget {
  final IconData icon;
  final String? label;
  final VoidCallback onTap;
  final Color? color;

  const _ActionButton({
    required this.icon,
    required this.onTap,
    this.label,
    this.color,
  });

  @override
  Widget build(BuildContext context) {
    final effectiveColor = color ?? context.colors.onSurfaceVariant;
    return InkWell(
      onTap: onTap,
      borderRadius: BorderRadius.circular(Radii.md),
      child: Padding(
        padding: const EdgeInsets.symmetric(
          horizontal: Grid.gutter,
          vertical: Grid.half,
        ),
        child: Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            Icon(icon, size: 16, color: effectiveColor),
            if (label != null) ...[
              const SizedBox(width: 3),
              Text(
                label!,
                style: context.textTheme.labelSmall?.copyWith(
                  color: effectiveColor,
                  fontWeight: FontWeight.w600,
                ),
              ),
            ],
          ],
        ),
      ),
    );
  }
}

String _shareUri(UserNote note) =>
    'nostr:${nostr.Nip19.encodeShareableIdentifiers(prefix: nostr.Nip19Prefix.nevent, data: note.id, author: note.pubkey, kind: 1)}';

String _shortPubkey(String pubkey) =>
    pubkey.length <= 8 ? pubkey : '${pubkey.substring(0, 8)}…';

String formatPulseRelativeTime(int createdAt) {
  final date = DateTime.fromMillisecondsSinceEpoch(createdAt * 1000);
  final diff = DateTime.now().difference(date);
  if (diff.inMinutes < 1) return 'just now';
  if (diff.inHours < 1) return '${diff.inMinutes}m';
  if (diff.inDays < 1) return '${diff.inHours}h';
  if (diff.inDays < 7) return '${diff.inDays}d';
  const months = [
    'Jan',
    'Feb',
    'Mar',
    'Apr',
    'May',
    'Jun',
    'Jul',
    'Aug',
    'Sep',
    'Oct',
    'Nov',
    'Dec',
  ];
  return '${months[date.month - 1]} ${date.day}';
}
