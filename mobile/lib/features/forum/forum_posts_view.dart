import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';

import '../../shared/relay/relay_closed_policy.dart';
import '../../shared/theme/theme.dart';
import '../../shared/widgets/buzz_loading_indicator.dart';
import '../../shared/widgets/frosted_app_bar.dart';
import '../../shared/widgets/load_error_view.dart';
import '../../shared/widgets/bee_refresh_indicator.dart';
import '../channels/channel.dart';
import '../channels/compose_bar.dart';
import 'forum_models.dart';
import 'forum_post_card.dart';
import 'forum_provider.dart';
import 'forum_thread_page.dart';

/// Main forum view — replaces the old _ForumPlaceholder.
///
/// Shows a list of forum posts for the channel with a FAB to open the compose
/// bar, and navigates to [ForumThreadPage] when a post is tapped.
class ForumPostsView extends HookConsumerWidget {
  final Channel channel;
  final String? currentPubkey;

  const ForumPostsView({
    super.key,
    required this.channel,
    required this.currentPubkey,
  });

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final postsAsync = ref.watch(forumPostsProvider(channel.id));
    final isComposing = useState(false);
    // A queued attachment can finish after this view is popped. Capture the
    // app-level provider container instead of retaining the route's WidgetRef.
    final providerContainer = ProviderScope.containerOf(context, listen: false);
    final forumDelivery = ForumEventDelivery.capture(providerContainer);

    // Periodic refresh (every 15s, matching desktop). A settled deadline
    // pauses polling; reopening the forum is the explicit retry.
    useEffect(() {
      final provider = forumPostsProvider(channel.id);
      // Only a deadline already settled when this surface mounts is a
      // reopen; a first load that fails after mounting is not retried.
      // Hooks run effects during build, so defer the invalidation past it.
      if (isSettledRelayDeadline(ref.read(provider))) {
        Future.microtask(() {
          if (context.mounted) ref.invalidate(provider);
        });
      }
      final timer = Stream.periodic(const Duration(seconds: 15)).listen((_) {
        if (!isSettledRelayDeadline(ref.read(provider))) {
          ref.invalidate(provider);
        }
      });
      return timer.cancel;
    }, [channel.id]);

    final canPost = channel.isMember && !channel.isArchived;

    return Column(
      children: [
        Expanded(
          child: Scaffold(
            // Transparent so the parent Scaffold's background shows through.
            backgroundColor: Colors.transparent,
            floatingActionButton: canPost && !isComposing.value
                ? FloatingActionButton(
                    heroTag: 'forum-fab',
                    onPressed: () => isComposing.value = true,
                    tooltip: 'New post',
                    shape: const CircleBorder(),
                    child: const Icon(LucideIcons.plus),
                  )
                : null,
            body: postsAsync.when(
              // A retry from an error shows loading, not the stale error and its Retry.
              skipLoadingOnRefresh: !postsAsync.hasError,
              loading: () => Padding(
                padding: EdgeInsets.only(top: frostedAppBarHeight(context)),
                child: const Center(
                  child: BuzzLoadingIndicator(
                    size: 44,
                    semanticLabel: 'Loading posts',
                  ),
                ),
              ),
              error: (e, _) => Padding(
                padding: EdgeInsets.only(top: frostedAppBarHeight(context)),
                child: LoadErrorView(
                  message: 'Failed to load posts',
                  onRetry: () => ref.invalidate(forumPostsProvider(channel.id)),
                ),
              ),
              data: (response) {
                final posts = response.posts;
                if (posts.isEmpty) {
                  return _EmptyState(
                    isMember: channel.isMember,
                    isArchived: channel.isArchived,
                  );
                }
                return BeeRefreshIndicator(
                  onRefresh: () async {
                    ref.invalidate(forumPostsProvider(channel.id));
                    await ref.read(forumPostsProvider(channel.id).future);
                  },
                  child: ListView.separated(
                    padding: EdgeInsets.only(
                      top: frostedAppBarHeight(context),
                      left: Grid.gutter,
                      right: Grid.gutter,
                      bottom: Grid.xs,
                    ),
                    itemCount: posts.length,
                    separatorBuilder: (_, _) =>
                        const SizedBox(height: Grid.xxs),
                    itemBuilder: (context, index) {
                      final post = posts[index];
                      return ForumPostCard(
                        post: post,
                        currentPubkey: currentPubkey,
                        onTap: () => _openThread(context, post),
                        onDelete: (eventId) async {
                          await deleteForumEvent(
                            ref,
                            channelId: channel.id,
                            eventId: eventId,
                          );
                        },
                      );
                    },
                  ),
                );
              },
            ),
          ),
        ),
        if (isComposing.value) ...[
          Align(
            alignment: Alignment.centerRight,
            child: Padding(
              padding: const EdgeInsets.only(right: Grid.xxs),
              child: IconButton(
                onPressed: () => isComposing.value = false,
                icon: const Icon(LucideIcons.x, size: 18),
                tooltip: 'Dismiss',
                visualDensity: VisualDensity.compact,
              ),
            ),
          ),
          ComposeBar(
            channelId: channel.id,
            hintText: 'Write your post\u2026',
            onSend:
                (
                  content,
                  mentionPubkeys, {
                  mediaTags = const <List<String>>[],
                }) async {
                  await forumDelivery.createPost(
                    channelId: channel.id,
                    content: content,
                    mentionPubkeys: mentionPubkeys,
                    mediaTags: mediaTags,
                  );
                  if (context.mounted) isComposing.value = false;
                },
          ),
        ],
      ],
    );
  }

  void _openThread(BuildContext context, ForumPost post) {
    Navigator.of(context).push(
      MaterialPageRoute<void>(
        builder: (_) => ForumThreadPage(
          channelId: channel.id,
          postEventId: post.eventId,
          currentPubkey: currentPubkey,
          isMember: channel.isMember,
          isArchived: channel.isArchived,
        ),
      ),
    );
  }
}

class _EmptyState extends StatelessWidget {
  final bool isMember;
  final bool isArchived;

  const _EmptyState({required this.isMember, required this.isArchived});

  @override
  Widget build(BuildContext context) {
    return Center(
      child: Padding(
        padding: const EdgeInsets.symmetric(horizontal: Grid.sm),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            Icon(
              LucideIcons.messageSquareText,
              size: Grid.xl,
              color: context.colors.onSurfaceVariant,
            ),
            const SizedBox(height: Grid.xxs),
            Text(
              'No posts yet',
              style: context.textTheme.bodyLarge?.copyWith(
                color: context.colors.onSurfaceVariant,
              ),
            ),
            const SizedBox(height: Grid.half),
            Text(
              isArchived
                  ? 'This forum is archived.'
                  : isMember
                  ? 'Start a discussion by creating the first post.'
                  : 'Join this forum to create posts.',
              style: context.textTheme.bodySmall?.copyWith(
                color: context.colors.onSurfaceVariant,
              ),
              textAlign: TextAlign.center,
            ),
          ],
        ),
      ),
    );
  }
}
