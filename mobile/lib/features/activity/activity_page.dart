import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';

import '../../shared/theme/theme.dart';
import '../../shared/widgets/filter_chip_bar.dart';
import '../../shared/widgets/frosted_app_bar.dart';
import '../../shared/widgets/frosted_scaffold.dart';
import '../channels/channel.dart';
import '../channels/channel_detail_page.dart';
import '../channels/message_content.dart';
import '../channels/channels_provider.dart';
import '../channels/small_avatar.dart';
import '../profile/user_cache_provider.dart';
import 'activity_provider.dart';
import 'feed_item.dart';

enum _Filter { all, mentions, needsAction, activity, agents }

class ActivityPage extends HookConsumerWidget {
  const ActivityPage({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final feedAsync = ref.watch(activityProvider);
    final channelsAsync = ref.watch(channelsProvider);
    final filter = useState(_Filter.all);

    // Cache last successful feed so the UI doesn't flash on rebuild.
    final cachedFeed = useRef<HomeFeedResponse?>(null);
    if (feedAsync.asData?.value case final data?) {
      cachedFeed.value = data;
    }
    final feed = cachedFeed.value;

    final Widget body;
    if (feed != null && !feed.isEmpty) {
      final items = _filteredItems(feed, filter.value);
      final channels = channelsAsync.asData?.value ?? [];

      // Preload user profiles for visible feed items.
      final pubkeys = items.map((i) => i.pubkey).toSet().toList();
      ref.read(userCacheProvider.notifier).preload(pubkeys);

      body = Column(
        children: [
          FilterChipBar<_Filter>(
            selected: filter.value,
            onSelected: (f) => filter.value = f,
            items: [
              const FilterChipItem(id: _Filter.all, label: 'All'),
              FilterChipItem(
                id: _Filter.mentions,
                label: 'Mentions',
                icon: LucideIcons.atSign,
                count: feed.mentions.length,
              ),
              FilterChipItem(
                id: _Filter.needsAction,
                label: 'Action',
                icon: LucideIcons.circleAlert,
                count: feed.needsAction.length,
              ),
              FilterChipItem(
                id: _Filter.activity,
                label: 'Activity',
                icon: LucideIcons.activity,
                count: feed.activity.length,
              ),
              FilterChipItem(
                id: _Filter.agents,
                label: 'Agents',
                icon: LucideIcons.bot,
                count: feed.agentActivity.length,
              ),
            ],
          ),
          Expanded(
            child: items.isEmpty
                ? _EmptyFilterState(filter: filter.value)
                : RefreshIndicator(
                    onRefresh: () =>
                        ref.read(activityProvider.notifier).refresh(),
                    child: ListView.separated(
                      padding: const EdgeInsets.symmetric(vertical: Grid.xxs),
                      itemCount: items.length,
                      separatorBuilder: (_, _) =>
                          const SizedBox(height: Grid.half),
                      itemBuilder: (context, index) {
                        final item = items[index];
                        return _FeedItemTile(
                          item: item,
                          onTap: () => _openItem(context, item, channels),
                        );
                      },
                    ),
                  ),
          ),
        ],
      );
    } else if (feedAsync.hasError) {
      body = _ErrorView(
        onRetry: () => ref.read(activityProvider.notifier).refresh(),
      );
    } else if (feedAsync.hasValue) {
      body = const _EmptyState();
    } else {
      body = const _LoadingSkeleton();
    }

    return FrostedScaffold(
      appBar: const FrostedAppBar(title: Text('Activity')),
      body: SafeArea(
        top: false,
        child: Padding(
          padding: EdgeInsets.only(top: frostedAppBarHeight(context)),
          child: body,
        ),
      ),
    );
  }

  List<FeedItem> _filteredItems(HomeFeedResponse feed, _Filter filter) {
    return switch (filter) {
      _Filter.all => feed.all,
      _Filter.mentions => feed.mentions,
      _Filter.needsAction => feed.needsAction,
      _Filter.activity => feed.activity,
      _Filter.agents => feed.agentActivity,
    };
  }

  void _openItem(BuildContext context, FeedItem item, List<Channel> channels) {
    if (item.channelId == null) return;
    final channel = channels
        .where((c) => c.id == item.channelId)
        .cast<Channel?>()
        .firstOrNull;
    if (channel == null) return;
    Navigator.of(context).push(
      MaterialPageRoute<void>(
        builder: (_) => ChannelDetailPage(channel: channel),
      ),
    );
  }
}

class _FeedItemTile extends ConsumerWidget {
  final FeedItem item;
  final VoidCallback onTap;

  const _FeedItemTile({required this.item, required this.onTap});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final userCache = ref.watch(userCacheProvider);
    final profile = userCache[item.pubkey.toLowerCase()];
    final authorLabel = profile?.label ?? _shortPubkey(item.pubkey);

    return InkWell(
      onTap: item.channelId != null ? onTap : null,
      child: Padding(
        padding: const EdgeInsets.symmetric(
          horizontal: Grid.xs,
          vertical: Grid.twelve,
        ),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            // Header row: icon + headline + author + channel + time
            Row(
              children: [
                Expanded(
                  child: Row(
                    children: [
                      Icon(
                        _categoryIcon(item.category),
                        size: 14,
                        color: context.colors.primary,
                      ),
                      const SizedBox(width: Grid.half),
                      Text(
                        item.headline,
                        style: context.textTheme.labelMedium?.copyWith(
                          fontWeight: FontWeight.w600,
                        ),
                      ),
                      const SizedBox(width: Grid.xxs),
                      SmallAvatar(pubkey: item.pubkey, userCache: userCache),
                      const SizedBox(width: Grid.quarter),
                      Flexible(
                        child: Text(
                          authorLabel,
                          style: context.textTheme.labelSmall?.copyWith(
                            color: context.colors.onSurfaceVariant,
                          ),
                          overflow: TextOverflow.ellipsis,
                        ),
                      ),
                      if (item.channelName.isNotEmpty) ...[
                        const SizedBox(width: Grid.half),
                        Flexible(
                          child: Text(
                            '#${item.channelName}',
                            style: context.textTheme.labelSmall?.copyWith(
                              color: context.colors.primary.withValues(
                                alpha: 0.8,
                              ),
                            ),
                            overflow: TextOverflow.ellipsis,
                          ),
                        ),
                      ],
                    ],
                  ),
                ),
                const SizedBox(width: Grid.xxs),
                Text(
                  _relativeTime(item.createdAt),
                  style: context.textTheme.labelSmall?.copyWith(
                    color: context.colors.onSurfaceVariant,
                  ),
                ),
              ],
            ),
            const SizedBox(height: Grid.half),
            // Content preview (max 2 lines)
            MessageContent(
              content: item.displayContent,
              tags: item.tags,
              maxLines: 2,
              baseStyle: context.textTheme.bodySmall?.copyWith(
                color: context.colors.onSurfaceVariant,
              ),
            ),
          ],
        ),
      ),
    );
  }

  static IconData _categoryIcon(String category) {
    return switch (category) {
      'mention' => LucideIcons.atSign,
      'needs_action' => LucideIcons.circleAlert,
      'agent_activity' => LucideIcons.bot,
      _ => LucideIcons.activity,
    };
  }

  static String _shortPubkey(String pk) =>
      pk.length >= 8 ? '${pk.substring(0, 8)}...' : pk;

  static String _relativeTime(int unixSeconds) {
    final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
    final diff = now - unixSeconds;

    if (diff < 60) return 'now';
    if (diff < 3600) return '${diff ~/ 60}m';
    if (diff < 86400) return '${diff ~/ 3600}h';
    if (diff < 604800) return '${diff ~/ 86400}d';
    final date = DateTime.fromMillisecondsSinceEpoch(unixSeconds * 1000);
    return '${date.month}/${date.day}';
  }
}

class _LoadingSkeleton extends StatelessWidget {
  const _LoadingSkeleton();

  @override
  Widget build(BuildContext context) {
    return ListView.separated(
      padding: const EdgeInsets.all(Grid.xs),
      itemCount: 8,
      separatorBuilder: (_, _) => const SizedBox(height: Grid.xs),
      itemBuilder: (context, _) => Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Container(
            width: 180,
            height: 12,
            decoration: BoxDecoration(
              color: context.colors.outlineVariant.withValues(alpha: 0.4),
              borderRadius: BorderRadius.circular(4),
            ),
          ),
          const SizedBox(height: Grid.xxs),
          Container(
            width: double.infinity,
            height: 10,
            decoration: BoxDecoration(
              color: context.colors.outlineVariant.withValues(alpha: 0.3),
              borderRadius: BorderRadius.circular(4),
            ),
          ),
          const SizedBox(height: Grid.half),
          Container(
            width: 240,
            height: 10,
            decoration: BoxDecoration(
              color: context.colors.outlineVariant.withValues(alpha: 0.2),
              borderRadius: BorderRadius.circular(4),
            ),
          ),
        ],
      ),
    );
  }
}

class _EmptyState extends StatelessWidget {
  const _EmptyState();

  @override
  Widget build(BuildContext context) {
    return Center(
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          Icon(
            LucideIcons.bell,
            size: Grid.xl,
            color: context.colors.onSurfaceVariant,
          ),
          const SizedBox(height: Grid.xs),
          Text(
            'No activity yet',
            style: context.textTheme.bodyLarge?.copyWith(
              color: context.colors.onSurfaceVariant,
            ),
          ),
          const SizedBox(height: Grid.half),
          Text(
            'Mentions, replies, and reactions will show up here.',
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

class _EmptyFilterState extends StatelessWidget {
  final _Filter filter;

  const _EmptyFilterState({required this.filter});

  @override
  Widget build(BuildContext context) {
    final (icon, message) = switch (filter) {
      _Filter.mentions => (LucideIcons.atSign, 'No mentions yet'),
      _Filter.needsAction => (
        LucideIcons.circleAlert,
        'Nothing needs your action',
      ),
      _Filter.activity => (LucideIcons.activity, 'No recent channel activity'),
      _Filter.agents => (LucideIcons.bot, 'No agent updates'),
      _Filter.all => (LucideIcons.bell, 'No activity yet'),
    };

    return Center(
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          Icon(icon, size: Grid.lg, color: context.colors.onSurfaceVariant),
          const SizedBox(height: Grid.xxs),
          Text(
            message,
            style: context.textTheme.bodyMedium?.copyWith(
              color: context.colors.onSurfaceVariant,
            ),
          ),
        ],
      ),
    );
  }
}

class _ErrorView extends StatelessWidget {
  final VoidCallback onRetry;

  const _ErrorView({required this.onRetry});

  @override
  Widget build(BuildContext context) {
    return Center(
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          Icon(
            LucideIcons.triangleAlert,
            size: Grid.lg,
            color: context.colors.error,
          ),
          const SizedBox(height: Grid.xxs),
          Text(
            'Failed to load activity',
            style: context.textTheme.bodyMedium?.copyWith(
              color: context.colors.onSurfaceVariant,
            ),
          ),
          const SizedBox(height: Grid.xs),
          FilledButton.icon(
            onPressed: onRetry,
            icon: const Icon(LucideIcons.refreshCcw, size: 16),
            label: const Text('Retry'),
          ),
        ],
      ),
    );
  }
}
