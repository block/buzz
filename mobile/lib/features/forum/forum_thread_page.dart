import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';

import '../../shared/theme/theme.dart';
import '../../shared/widgets/frosted_app_bar.dart';
import '../../shared/widgets/frosted_scaffold.dart';
import '../channels/compose_bar.dart';
import '../channels/message_content.dart';
import '../profile/user_cache_provider.dart';
import '../profile/user_profile.dart';
import '../profile/user_profile_sheet.dart';
import 'forum_models.dart';
import 'forum_provider.dart';

/// Full-screen page showing a forum post and its replies.
class ForumThreadPage extends HookConsumerWidget {
  final String channelId;
  final String postEventId;
  final String? currentPubkey;
  final bool isMember;
  final bool isArchived;

  const ForumThreadPage({
    super.key,
    required this.channelId,
    required this.postEventId,
    required this.currentPubkey,
    required this.isMember,
    required this.isArchived,
  });

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final threadAsync = ref.watch(
      forumThreadProvider((channelId: channelId, eventId: postEventId)),
    );

    // Periodic refresh (every 10s, matching desktop).
    useEffect(() {
      final timer = Stream.periodic(const Duration(seconds: 10)).listen((_) {
        ref.invalidate(
          forumThreadProvider((channelId: channelId, eventId: postEventId)),
        );
      });
      return timer.cancel;
    }, [channelId, postEventId]);

    final isOwnPost =
        threadAsync
            .whenData(
              (t) =>
                  currentPubkey != null &&
                  t.post.pubkey.toLowerCase() == currentPubkey!.toLowerCase(),
            )
            .value ??
        false;

    return FrostedScaffold(
      appBar: FrostedAppBar(
        title: const Text('Thread'),
        actions: [
          if (isOwnPost)
            IconButton(
              onPressed: () =>
                  _showPostActions(context, ref, threadAsync.value!),
              tooltip: 'Post actions',
              icon: const Icon(LucideIcons.ellipsis),
            ),
        ],
      ),
      body: threadAsync.when(
        loading: () => Padding(
          padding: EdgeInsets.only(top: frostedAppBarHeight(context)),
          child: const Center(child: CircularProgressIndicator()),
        ),
        error: (e, _) => Padding(
          padding: EdgeInsets.only(top: frostedAppBarHeight(context)),
          child: Center(
            child: Text(
              'Failed to load thread',
              style: context.textTheme.bodyMedium?.copyWith(
                color: context.colors.error,
              ),
            ),
          ),
        ),
        data: (thread) => _ThreadContent(
          thread: thread,
          channelId: channelId,
          currentPubkey: currentPubkey,
          isMember: isMember,
          isArchived: isArchived,
        ),
      ),
    );
  }

  void _showPostActions(
    BuildContext context,
    WidgetRef ref,
    ForumThreadResponse thread,
  ) {
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
                  Clipboard.setData(ClipboardData(text: thread.post.content));
                },
              ),
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
                  _confirmDeletePost(context, ref, thread.post.eventId);
                },
              ),
            ],
          ),
        ),
      ),
    );
  }

  void _confirmDeletePost(BuildContext context, WidgetRef ref, String eventId) {
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
            onPressed: () async {
              Navigator.of(dialogContext).pop();
              await deleteForumEvent(
                ref,
                channelId: channelId,
                eventId: eventId,
              );
              if (context.mounted) {
                Navigator.of(context).pop();
              }
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

class _ThreadContent extends HookConsumerWidget {
  final ForumThreadResponse thread;
  final String channelId;
  final String? currentPubkey;
  final bool isMember;
  final bool isArchived;

  const _ThreadContent({
    required this.thread,
    required this.channelId,
    required this.currentPubkey,
    required this.isMember,
    required this.isArchived,
  });

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final post = thread.post;
    final replies = thread.replies;

    // Preload profiles for all participants.
    final allPubkeys = useMemoized(() {
      final pks = <String>{post.pubkey};
      for (final reply in replies) {
        pks.add(reply.pubkey);
      }
      return pks.toList();
    }, [post, replies]);

    useEffect(() {
      if (allPubkeys.isNotEmpty) {
        ref.read(userCacheProvider.notifier).preload(allPubkeys);
      }
      return null;
    }, [allPubkeys]);

    return Column(
      children: [
        Expanded(
          child: ListView(
            padding: EdgeInsets.only(
              top: frostedAppBarHeight(context),
              bottom: Grid.xs,
            ),
            children: [
              _OriginalPost(post: post),

              Padding(
                padding: const EdgeInsets.symmetric(
                  horizontal: Grid.xs,
                  vertical: Grid.xxs,
                ),
                child: Row(
                  children: [
                    Icon(
                      LucideIcons.messageSquare,
                      size: 16,
                      color: context.colors.onSurfaceVariant,
                    ),
                    const SizedBox(width: Grid.half),
                    Text(
                      '${replies.length} ${replies.length == 1 ? 'reply' : 'replies'}',
                      style: context.textTheme.labelMedium?.copyWith(
                        color: context.colors.onSurfaceVariant,
                        fontWeight: FontWeight.w600,
                      ),
                    ),
                  ],
                ),
              ),

              // Reply list
              if (replies.isEmpty)
                Padding(
                  padding: const EdgeInsets.all(Grid.sm),
                  child: Text(
                    'No replies yet. Be the first to respond.',
                    style: context.textTheme.bodyMedium?.copyWith(
                      color: context.colors.onSurfaceVariant,
                    ),
                    textAlign: TextAlign.center,
                  ),
                )
              else
                for (final reply in replies)
                  _ReplyRow(
                    reply: reply,
                    currentPubkey: currentPubkey,
                    channelId: channelId,
                    rootEventId: post.eventId,
                  ),
            ],
          ),
        ),

        // Reply composer
        if (isMember && !isArchived)
          ComposeBar(
            channelId: channelId,
            hintText: 'Reply to this post\u2026',
            onSend:
                (
                  content,
                  mentionPubkeys, {
                  mediaTags = const <List<String>>[],
                }) => createForumReply(
                  ref,
                  channelId: channelId,
                  parentEventId: post.eventId,
                  content: content,
                  mentionPubkeys: mentionPubkeys,
                  mediaTags: mediaTags,
                ),
          ),
      ],
    );
  }
}

class _OriginalPost extends ConsumerWidget {
  final ForumPost post;

  const _OriginalPost({required this.post});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final pk = post.pubkey.toLowerCase();
    final profile =
        ref.watch(userCacheProvider.select((cache) => cache[pk])) ??
        ref.read(userCacheProvider.notifier).get(pk);
    final displayName = profile?.label ?? _shortPubkey(post.pubkey);

    final userCache = ref.watch(userCacheProvider);
    final mentionNames = _buildMentionNames(post.mentionPubkeys, userCache);

    return Padding(
      padding: const EdgeInsets.all(Grid.xs),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Row(
            children: [
              GestureDetector(
                onTap: () => showUserProfileSheet(context, post.pubkey),
                child: _Avatar(
                  profile: profile,
                  pubkey: post.pubkey,
                  radius: 16,
                ),
              ),
              const SizedBox(width: Grid.xxs),
              Expanded(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    GestureDetector(
                      onTap: () => showUserProfileSheet(context, post.pubkey),
                      child: Text(
                        displayName,
                        style: context.textTheme.labelMedium?.copyWith(
                          fontWeight: FontWeight.w600,
                        ),
                      ),
                    ),
                    Text(
                      formatRelativeTime(post.createdAt),
                      style: context.textTheme.labelSmall?.copyWith(
                        color: context.colors.onSurfaceVariant,
                      ),
                    ),
                  ],
                ),
              ),
            ],
          ),
          const SizedBox(height: Grid.xxs),
          MessageContent(
            content: post.content,
            mentionNames: mentionNames,
            tags: post.tags,
          ),
        ],
      ),
    );
  }
}

class _ReplyRow extends ConsumerWidget {
  final ThreadReply reply;
  final String? currentPubkey;
  final String channelId;
  final String rootEventId;

  const _ReplyRow({
    required this.reply,
    required this.currentPubkey,
    required this.channelId,
    required this.rootEventId,
  });

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final pk = reply.pubkey.toLowerCase();
    final profile =
        ref.watch(userCacheProvider.select((cache) => cache[pk])) ??
        ref.read(userCacheProvider.notifier).get(pk);
    final displayName = profile?.label ?? _shortPubkey(reply.pubkey);

    final userCache = ref.watch(userCacheProvider);
    final mentionNames = _buildMentionNames(reply.mentionPubkeys, userCache);

    return Padding(
      padding: const EdgeInsets.symmetric(
        horizontal: Grid.xs,
        vertical: Grid.xxs,
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Row(
            children: [
              GestureDetector(
                onTap: () => showUserProfileSheet(context, reply.pubkey),
                child: _Avatar(
                  profile: profile,
                  pubkey: reply.pubkey,
                  radius: 12,
                ),
              ),
              const SizedBox(width: Grid.xxs),
              Expanded(
                child: Row(
                  children: [
                    GestureDetector(
                      onTap: () => showUserProfileSheet(context, reply.pubkey),
                      child: Text(
                        displayName,
                        style: context.textTheme.labelMedium?.copyWith(
                          fontWeight: FontWeight.w600,
                        ),
                      ),
                    ),
                    const SizedBox(width: Grid.xxs),
                    Text(
                      formatRelativeTime(reply.createdAt),
                      style: context.textTheme.labelSmall?.copyWith(
                        color: context.colors.onSurfaceVariant,
                      ),
                    ),
                  ],
                ),
              ),
              SizedBox(
                width: 28,
                height: 28,
                child: IconButton(
                  onPressed: () => _showActions(context, ref),
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
          Padding(
            padding: const EdgeInsets.only(left: 32, top: Grid.half),
            child: MessageContent(
              content: reply.content,
              mentionNames: mentionNames,
              tags: reply.tags,
            ),
          ),
        ],
      ),
    );
  }

  void _showActions(BuildContext context, WidgetRef ref) {
    final isOwn =
        currentPubkey != null &&
        reply.pubkey.toLowerCase() == currentPubkey!.toLowerCase();

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
                  Clipboard.setData(ClipboardData(text: reply.content));
                },
              ),
              if (isOwn)
                ListTile(
                  leading: Icon(
                    LucideIcons.trash2,
                    color: Theme.of(sheetContext).colorScheme.error,
                  ),
                  title: Text(
                    'Delete reply',
                    style: TextStyle(
                      color: Theme.of(sheetContext).colorScheme.error,
                    ),
                  ),
                  onTap: () {
                    Navigator.of(sheetContext).pop();
                    _confirmDelete(context, ref);
                  },
                ),
            ],
          ),
        ),
      ),
    );
  }

  void _confirmDelete(BuildContext context, WidgetRef ref) {
    showDialog<void>(
      context: context,
      builder: (dialogContext) => AlertDialog(
        title: const Text('Delete reply'),
        content: const Text('This cannot be undone.'),
        actions: [
          TextButton(
            onPressed: () => Navigator.of(dialogContext).pop(),
            child: const Text('Cancel'),
          ),
          FilledButton(
            onPressed: () async {
              Navigator.of(dialogContext).pop();
              await deleteForumEvent(
                ref,
                channelId: channelId,
                eventId: reply.eventId,
                rootEventId: rootEventId,
              );
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

class _Avatar extends StatelessWidget {
  final UserProfile? profile;
  final String pubkey;
  final double radius;

  const _Avatar({
    required this.profile,
    required this.pubkey,
    required this.radius,
  });

  @override
  Widget build(BuildContext context) {
    final initial =
        profile?.initial ?? (pubkey.isNotEmpty ? pubkey[0].toUpperCase() : '?');
    final avatarUrl = profile?.avatarUrl;

    return CircleAvatar(
      radius: radius,
      backgroundColor: context.colors.primaryContainer,
      backgroundImage: avatarUrl != null ? NetworkImage(avatarUrl) : null,
      child: avatarUrl == null
          ? Text(
              initial,
              style: TextStyle(
                fontSize: radius * 0.75,
                fontWeight: FontWeight.w600,
                color: context.colors.onPrimaryContainer,
              ),
            )
          : null,
    );
  }
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

String _shortPubkey(String pubkey) {
  if (pubkey.length > 12) return '${pubkey.substring(0, 8)}\u2026';
  return pubkey;
}
