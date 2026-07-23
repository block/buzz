import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';

import '../../shared/theme/theme.dart';
import '../../shared/widgets/avatar_image.dart';
import '../../shared/widgets/filter_chip_bar.dart';
import '../../shared/widgets/frosted_app_bar.dart';
import '../../shared/widgets/frosted_scaffold.dart';
import '../channels/channel.dart';
import '../channels/channel_detail_page.dart';
import '../channels/channel_management_provider.dart';
import '../channels/channels_provider.dart';
import '../channels/small_avatar.dart';
import '../channels/message_content.dart';
import '../channels/date_formatters.dart';
import '../forum/forum_thread_page.dart';
import '../profile/profile_provider.dart';
import '../profile/user_cache_provider.dart';
import '../profile/user_profile.dart';
import 'search_provider.dart';

enum _SearchFilter { all, messages, channels, people }

class SearchPage extends HookConsumerWidget {
  const SearchPage({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final searchState = ref.watch(searchProvider);
    final currentPubkey = ref
        .watch(profileProvider)
        .whenData((value) => value?.pubkey)
        .value;
    final activeFilter = useState(_SearchFilter.all);
    final textController = useTextEditingController();
    final hasText = useListenableSelector(
      textController,
      () => textController.text.isNotEmpty,
    );

    return FrostedScaffold(
      resizeToAvoidBottomInset: true,
      appBar: FrostedAppBar(
        title: Container(
          height: 36,
          padding: const EdgeInsets.symmetric(horizontal: Grid.half),
          decoration: BoxDecoration(
            color: context.colors.surfaceContainerHighest,
            borderRadius: BorderRadius.circular(Radii.lg),
          ),
          child: TextField(
            controller: textController,
            decoration: InputDecoration(
              hintText: 'Search messages, channels, people\u2026',
              hintStyle: context.textTheme.bodyMedium?.copyWith(
                color: context.colors.onSurfaceVariant,
              ),
              prefixIcon: const Icon(LucideIcons.search, size: 16),
              prefixIconConstraints: const BoxConstraints(minWidth: 32),
              border: InputBorder.none,
              enabledBorder: InputBorder.none,
              focusedBorder: InputBorder.none,
              isDense: true,
              contentPadding: const EdgeInsets.symmetric(vertical: Grid.xxs),
            ),
            style: context.textTheme.bodyMedium,
            onChanged: (value) =>
                ref.read(searchProvider.notifier).search(value),
          ),
        ),
        actions: [
          if (hasText)
            IconButton(
              icon: const Icon(LucideIcons.x, size: 20),
              onPressed: () {
                textController.clear();
                ref.read(searchProvider.notifier).clear();
              },
            ),
        ],
      ),
      body: Column(
        children: [
          SizedBox(height: frostedAppBarHeight(context)),
          FilterChipBar<_SearchFilter>(
            selected: activeFilter.value,
            onSelected: (f) => activeFilter.value = f,
            items: [
              for (final f in _SearchFilter.values)
                FilterChipItem(id: f, label: f.label),
            ],
          ),
          Expanded(
            child: _SearchBody(
              state: searchState,
              filter: activeFilter.value,
              currentPubkey: currentPubkey,
            ),
          ),
        ],
      ),
    );
  }
}

class _SearchBody extends ConsumerWidget {
  final SearchState state;
  final _SearchFilter filter;
  final String? currentPubkey;

  const _SearchBody({
    required this.state,
    required this.filter,
    required this.currentPubkey,
  });

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    if (state.query.isEmpty) {
      return Center(
        child: Text(
          'Search messages, channels, and people',
          style: context.textTheme.bodyMedium?.copyWith(
            color: context.colors.onSurfaceVariant,
          ),
        ),
      );
    }

    final showChannels =
        filter == _SearchFilter.all || filter == _SearchFilter.channels;
    final showPeople =
        filter == _SearchFilter.all || filter == _SearchFilter.people;
    final showMessages =
        filter == _SearchFilter.all || filter == _SearchFilter.messages;

    final hasAnyResults =
        state.channelResults.isNotEmpty ||
        state.userResults.isNotEmpty ||
        state.messageResults.isNotEmpty;

    if (!state.isLoading && !hasAnyResults) {
      return Center(
        child: Text(
          "No results for '${state.query}'",
          style: context.textTheme.bodyMedium?.copyWith(
            color: context.colors.onSurfaceVariant,
          ),
        ),
      );
    }

    return ListView(
      padding: const EdgeInsets.only(bottom: Grid.xl),
      children: [
        if (showChannels && state.channelResults.isNotEmpty)
          _ChannelsSection(channels: state.channelResults),
        if (showPeople && state.userResults.isNotEmpty)
          _PeopleSection(users: state.userResults),
        if (showMessages && state.messageResults.isNotEmpty)
          _MessagesSection(
            hits: state.messageResults,
            currentPubkey: currentPubkey,
          ),
        if (state.isLoading)
          const Padding(
            padding: EdgeInsets.all(Grid.sm),
            child: Center(child: CircularProgressIndicator()),
          ),
      ],
    );
  }
}

class _ChannelsSection extends StatelessWidget {
  final List<Channel> channels;

  const _ChannelsSection({required this.channels});

  @override
  Widget build(BuildContext context) {
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        _SectionLabel(label: 'Channels'),
        for (final channel in channels)
          ListTile(
            leading: Icon(channelIcon(channel), size: 20),
            title: Text(channel.name),
            subtitle: Text(
              '${channel.memberCount} member${channel.memberCount == 1 ? '' : 's'}',
              style: context.textTheme.bodySmall?.copyWith(
                color: context.colors.onSurfaceVariant,
              ),
            ),
            trailing: !channel.isMember && !channel.isDm
                ? Container(
                    padding: const EdgeInsets.symmetric(
                      horizontal: Grid.half + 2,
                      vertical: 3,
                    ),
                    decoration: BoxDecoration(
                      color: context.colors.primary.withValues(alpha: 0.1),
                      borderRadius: BorderRadius.circular(Radii.sm),
                    ),
                    child: Text(
                      'Open',
                      style: context.textTheme.labelSmall?.copyWith(
                        color: context.colors.primary,
                        fontWeight: FontWeight.w600,
                      ),
                    ),
                  )
                : null,
            onTap: () => Navigator.of(context).push(
              MaterialPageRoute<void>(
                builder: (_) => ChannelDetailPage(channel: channel),
              ),
            ),
          ),
      ],
    );
  }
}

class _PeopleSection extends ConsumerWidget {
  final List<DirectoryUser> users;

  const _PeopleSection({required this.users});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        _SectionLabel(label: 'People'),
        for (final user in users)
          ListTile(
            leading: AvatarImage(
              imageUrl: user.avatarUrl,
              radius: 20,
              fallback: Text(user.label.substring(0, 1).toUpperCase()),
            ),
            title: Text(user.label),
            subtitle: Text(
              user.secondaryLabel,
              style: context.textTheme.bodySmall?.copyWith(
                color: context.colors.onSurfaceVariant,
              ),
            ),
            onTap: () async {
              final channel = await ref
                  .read(channelActionsProvider)
                  .openDm(pubkeys: [user.pubkey]);
              if (!context.mounted) return;
              await Navigator.of(context).push(
                MaterialPageRoute<void>(
                  builder: (_) => ChannelDetailPage(channel: channel),
                ),
              );
            },
          ),
      ],
    );
  }
}

class _MessagesSection extends ConsumerWidget {
  final List<SearchHit> hits;
  final String? currentPubkey;

  const _MessagesSection({required this.hits, required this.currentPubkey});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final profiles = ref.watch(userCacheProvider);
    final channels = ref.watch(channelsProvider).value ?? [];

    // Preload author profiles.
    final pubkeys = hits.map((h) => h.pubkey.toLowerCase()).toSet().toList();
    ref.read(userCacheProvider.notifier).preload(pubkeys);

    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        _SectionLabel(label: 'Messages'),
        for (final hit in hits)
          _MessageTile(
            hit: hit,
            authorProfile: profiles[hit.pubkey.toLowerCase()],
            userCache: profiles,
            channel: channels.where((c) => c.id == hit.channelId).firstOrNull,
            currentPubkey: currentPubkey,
          ),
      ],
    );
  }
}

class _MessageTile extends StatelessWidget {
  final SearchHit hit;
  final UserProfile? authorProfile;
  final Map<String, UserProfile> userCache;
  final Channel? channel;
  final String? currentPubkey;

  const _MessageTile({
    required this.hit,
    required this.authorProfile,
    required this.userCache,
    required this.channel,
    required this.currentPubkey,
  });

  @override
  Widget build(BuildContext context) {
    final authorName = authorProfile?.label ?? shortPubkey(hit.pubkey);
    final timeAgo = relativeTime(hit.createdAt);

    return ListTile(
      leading: SmallAvatar(pubkey: hit.pubkey, userCache: userCache),
      title: Row(
        children: [
          Expanded(
            child: Text(
              authorName,
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              style: context.textTheme.bodyMedium?.copyWith(
                fontWeight: FontWeight.w600,
              ),
            ),
          ),
          if (hit.channelName != null) ...[
            const SizedBox(width: Grid.half),
            Container(
              padding: const EdgeInsets.symmetric(
                horizontal: Grid.half,
                vertical: 2,
              ),
              decoration: BoxDecoration(
                color: context.colors.surfaceContainerHighest,
                borderRadius: BorderRadius.circular(Radii.sm),
              ),
              child: Text(
                hit.channelName!,
                style: context.textTheme.labelSmall?.copyWith(
                  color: context.colors.onSurfaceVariant,
                ),
              ),
            ),
          ],
        ],
      ),
      subtitle: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          const SizedBox(height: 2),
          MessageContent(
            content: hit.content,
            tags: hit.tags,
            maxLines: 2,
            baseStyle: context.textTheme.bodyMedium,
          ),
          const SizedBox(height: 2),
          Text(
            timeAgo,
            style: context.textTheme.labelSmall?.copyWith(
              color: context.colors.onSurfaceVariant,
            ),
          ),
        ],
      ),
      onTap: () => _navigateToHit(context, hit, channel),
    );
  }

  void _navigateToHit(BuildContext context, SearchHit hit, Channel? channel) {
    if (channel == null) return;

    if (hit.kind == 45001) {
      Navigator.of(context).push(
        MaterialPageRoute<void>(
          builder: (_) => ForumThreadPage(
            channelId: channel.id,
            postEventId: hit.eventId,
            currentPubkey: currentPubkey,
            isMember: channel.isMember,
            isArchived: channel.isArchived,
          ),
        ),
      );
    } else {
      Navigator.of(context).push(
        MaterialPageRoute<void>(
          builder: (_) => ChannelDetailPage(channel: channel),
        ),
      );
    }
  }
}

class _SectionLabel extends StatelessWidget {
  final String label;

  const _SectionLabel({required this.label});

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.fromLTRB(
        Grid.gutter,
        Grid.xs,
        Grid.gutter,
        Grid.half,
      ),
      child: Text(
        label.toUpperCase(),
        style: context.textTheme.labelSmall?.copyWith(
          color: context.colors.onSurfaceVariant,
          fontWeight: FontWeight.w600,
          letterSpacing: 0.8,
        ),
      ),
    );
  }
}

extension on _SearchFilter {
  String get label => switch (this) {
    _SearchFilter.all => 'All',
    _SearchFilter.messages => 'Messages',
    _SearchFilter.channels => 'Channels',
    _SearchFilter.people => 'People',
  };
}
