import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';

import '../../shared/theme/theme.dart';
import '../../shared/widgets/avatar_image.dart';
import '../../shared/widgets/filter_chip_bar.dart';
import '../../shared/widgets/frosted_app_bar.dart';
import '../../shared/widgets/frosted_scaffold.dart';
import '../../shared/widgets/message_author_meta.dart';
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
import 'recent_searches_provider.dart';
import 'search_provider.dart';

enum _SearchFilter { all, messages, channels, people }

const _searchFieldMinHeight = 36.0;
const _searchFieldVerticalPadding = Grid.xxs;
const _searchFieldHint = 'Search messages, channels, people\u2026';
const _searchCancelEnterDuration = Duration(milliseconds: 160);
const _searchCancelExitDuration = Duration(milliseconds: 120);

double _searchFieldHeight(BuildContext context) {
  const style = searchInputTextStyle;
  final scaledFontSize = MediaQuery.textScalerOf(
    context,
  ).scale(style.fontSize ?? 15);
  final contentHeight =
      scaledFontSize * (style.height ?? 1) + _searchFieldVerticalPadding * 2;
  return contentHeight > _searchFieldMinHeight
      ? contentHeight
      : _searchFieldMinHeight;
}

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
    final focusNode = useFocusNode();
    final isSearchFocused = useListenableSelector(
      focusNode,
      () => focusNode.hasFocus,
    );
    final reduceMotion = MediaQuery.disableAnimationsOf(context);
    final isBuzzTheme = context.appColors.topSectionGradient != null;
    final buzzSearchColor = context.theme.brightness == Brightness.dark
        ? Colors.white
        : Colors.black;
    final searchSurfaceColor = isBuzzTheme
        ? buzzSearchColor.withValues(alpha: 0.04)
        : context.colors.surfaceContainerHighest;
    final searchMutedColor = isBuzzTheme
        ? buzzSearchColor.withValues(alpha: 0.4)
        : context.colors.onSurfaceVariant;
    final headerTitleStyle = context.textTheme.titleMedium?.copyWith(
      fontSize: 22,
      fontWeight: FontWeight.w600,
    );
    final searchFieldHeight = _searchFieldHeight(context);
    final searchControlHeight = searchFieldHeight > Grid.xl
        ? searchFieldHeight
        : Grid.xl;
    final searchHeaderBottomHeight = searchControlHeight + Grid.twelve;

    void runRecentSearch(String query) {
      textController.value = TextEditingValue(
        text: query,
        selection: TextSelection.collapsed(offset: query.length),
      );
      focusNode.requestFocus();
      ref.read(recentSearchesProvider.notifier).record(query);
      ref.read(searchProvider.notifier).search(query);
    }

    return FrostedScaffold(
      // Keep the empty state centered in the page rather than the portion left
      // above the keyboard.
      resizeToAvoidBottomInset: false,
      appBar: FrostedAppBar(
        gradient: context.appColors.topSectionGradient,
        title: const Text('Search'),
        titleStyle: headerTitleStyle,
        bottomHeight: searchHeaderBottomHeight,
        bottom: Padding(
          padding: const EdgeInsets.fromLTRB(
            Grid.gutter,
            0,
            Grid.gutter,
            Grid.twelve,
          ),
          child: Row(
            children: [
              Expanded(
                child: Container(
                  key: const Key('search-field-container'),
                  height: searchFieldHeight,
                  padding: const EdgeInsets.symmetric(horizontal: Grid.half),
                  decoration: BoxDecoration(
                    color: searchSurfaceColor,
                    borderRadius: BorderRadius.circular(Radii.lg),
                  ),
                  child: TextField(
                    key: const Key('search-field'),
                    controller: textController,
                    focusNode: focusNode,
                    decoration: InputDecoration(
                      hintText: isSearchFocused ? null : _searchFieldHint,
                      hintStyle: searchInputTextStyle.copyWith(
                        color: searchMutedColor,
                      ),
                      prefixIcon: Icon(
                        LucideIcons.search,
                        size: 16,
                        color: searchMutedColor,
                      ),
                      prefixIconConstraints: const BoxConstraints(minWidth: 32),
                      border: InputBorder.none,
                      enabledBorder: InputBorder.none,
                      focusedBorder: InputBorder.none,
                      isDense: true,
                      contentPadding: const EdgeInsets.symmetric(
                        vertical: _searchFieldVerticalPadding,
                      ),
                    ),
                    style: searchInputTextStyle.copyWith(
                      color: context.colors.onSurface,
                    ),
                    textInputAction: TextInputAction.search,
                    onChanged: (value) =>
                        ref.read(searchProvider.notifier).search(value),
                    onSubmitted: (value) {
                      final query = value.trim();
                      if (query.isEmpty) return;
                      ref.read(recentSearchesProvider.notifier).record(query);
                    },
                  ),
                ),
              ),
              AnimatedSwitcher(
                duration: reduceMotion
                    ? Duration.zero
                    : _searchCancelEnterDuration,
                reverseDuration: reduceMotion
                    ? Duration.zero
                    : _searchCancelExitDuration,
                transitionBuilder: (child, animation) {
                  final curvedAnimation = CurvedAnimation(
                    parent: animation,
                    curve: Curves.easeOutCubic,
                    reverseCurve: Curves.easeInCubic,
                  );
                  return SizeTransition(
                    sizeFactor: curvedAnimation,
                    axis: Axis.horizontal,
                    axisAlignment: 1,
                    child: FadeTransition(
                      opacity: curvedAnimation,
                      child: SlideTransition(
                        position: Tween<Offset>(
                          begin: const Offset(0.35, 0),
                          end: Offset.zero,
                        ).animate(curvedAnimation),
                        child: child,
                      ),
                    ),
                  );
                },
                child: isSearchFocused
                    ? Padding(
                        key: const ValueKey('search-cancel-visible'),
                        padding: const EdgeInsets.only(left: Grid.xxs),
                        child: TextButton(
                          key: const Key('search-cancel'),
                          onPressed: () {
                            textController.clear();
                            ref.read(searchProvider.notifier).clear();
                            focusNode.unfocus();
                          },
                          style: TextButton.styleFrom(
                            foregroundColor: context.colors.primary,
                            minimumSize: Size(0, searchControlHeight),
                            padding: const EdgeInsets.symmetric(
                              horizontal: Grid.half,
                              vertical: Grid.xxs,
                            ),
                            tapTargetSize: MaterialTapTargetSize.shrinkWrap,
                          ),
                          child: Text(
                            'Cancel',
                            style: filterChipTextStyle.copyWith(
                              color: context.colors.primary,
                              fontWeight: FontWeight.w500,
                            ),
                          ),
                        ),
                      )
                    : const SizedBox.shrink(
                        key: ValueKey('search-cancel-hidden'),
                      ),
              ),
            ],
          ),
        ),
        actions: const [],
      ),
      body: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          SizedBox(
            height: frostedAppBarHeight(
              context,
              bottomHeight: searchHeaderBottomHeight,
              titleStyle: headerTitleStyle,
            ),
          ),
          FilterChipBar<_SearchFilter>(
            expandItems: true,
            visualDensity: const VisualDensity(horizontal: -2),
            chipVerticalPadding: Grid.xxs,
            barVerticalPadding: Grid.twelve,
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
              onRecentSearchSelected: runRecentSearch,
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
  final ValueChanged<String> onRecentSearchSelected;

  const _SearchBody({
    required this.state,
    required this.filter,
    required this.currentPubkey,
    required this.onRecentSearchSelected,
  });

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    if (state.query.isEmpty) {
      final recentSearches = ref.watch(recentSearchesProvider);
      if (recentSearches.isNotEmpty) {
        return _RecentSearches(
          searches: recentSearches,
          onSelected: onRecentSearchSelected,
          onClear: ref.read(recentSearchesProvider.notifier).clear,
        );
      }

      return Center(
        child: Padding(
          key: const Key('search-empty-state'),
          padding: const EdgeInsets.symmetric(horizontal: Grid.gutter),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              Icon(
                LucideIcons.search,
                size: 64,
                color: context.colors.onSurfaceVariant,
              ),
              const SizedBox(height: Grid.xs),
              Text(
                'Search messages, channels, and people',
                textAlign: TextAlign.center,
                style: context.textTheme.bodyMedium?.copyWith(
                  color: context.colors.onSurfaceVariant,
                ),
              ),
            ],
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
    void recordResultSelection() =>
        ref.read(recentSearchesProvider.notifier).record(state.query);

    if (!state.isLoading && !hasAnyResults) {
      return Padding(
        key: const Key('search-no-results-state'),
        padding: EdgeInsets.only(
          bottom: MediaQuery.viewInsetsOf(context).bottom,
        ),
        child: Center(
          child: Text(
            "No results for '${state.query}'",
            style: context.textTheme.bodyMedium?.copyWith(
              color: context.colors.onSurfaceVariant,
            ),
          ),
        ),
      );
    }

    return ListView(
      key: const Key('search-results-list'),
      padding: EdgeInsets.only(
        bottom: Grid.xl + MediaQuery.viewInsetsOf(context).bottom,
      ),
      children: [
        if (showChannels && state.channelResults.isNotEmpty)
          _ChannelsSection(
            channels: state.channelResults,
            onResultSelected: recordResultSelection,
          ),
        if (showPeople && state.userResults.isNotEmpty)
          _PeopleSection(
            users: state.userResults,
            onResultSelected: recordResultSelection,
          ),
        if (showMessages && state.messageResults.isNotEmpty)
          _MessagesSection(
            hits: state.messageResults,
            currentPubkey: currentPubkey,
            onResultSelected: recordResultSelection,
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

class _RecentSearches extends StatelessWidget {
  final List<String> searches;
  final ValueChanged<String> onSelected;
  final VoidCallback onClear;

  const _RecentSearches({
    required this.searches,
    required this.onSelected,
    required this.onClear,
  });

  @override
  Widget build(BuildContext context) {
    return ListView(
      key: const Key('recent-searches-list'),
      padding: EdgeInsets.only(
        bottom: Grid.xl + MediaQuery.viewInsetsOf(context).bottom,
      ),
      children: [
        Padding(
          padding: const EdgeInsets.fromLTRB(
            Grid.gutter,
            Grid.xs,
            Grid.xxs,
            Grid.half,
          ),
          child: Row(
            children: [
              Expanded(
                child: Text(
                  'Recent searches',
                  key: const Key('recent-searches-heading'),
                  style: activityContextTextStyle.copyWith(
                    color: context.colors.onSurfaceVariant,
                  ),
                ),
              ),
              TextButton(
                key: const Key('clear-recent-searches'),
                onPressed: onClear,
                child: Text(
                  'Clear',
                  style: activityContextTextStyle.copyWith(
                    color: context.colors.primary,
                  ),
                ),
              ),
            ],
          ),
        ),
        for (var index = 0; index < searches.length; index++)
          InkWell(
            key: ValueKey('recent-search-$index'),
            onTap: () => onSelected(searches[index]),
            child: ConstrainedBox(
              constraints: const BoxConstraints(minHeight: Grid.xl),
              child: Padding(
                padding: const EdgeInsets.symmetric(
                  horizontal: Grid.gutter,
                  vertical: Grid.twelve,
                ),
                child: Row(
                  children: [
                    Icon(
                      LucideIcons.clock,
                      size: 18,
                      color: context.colors.onSurfaceVariant,
                    ),
                    const SizedBox(width: Grid.twelve),
                    Expanded(
                      child: Text(
                        searches[index],
                        style: contentListTitleTextStyle.copyWith(
                          color: context.colors.onSurface,
                        ),
                      ),
                    ),
                    Icon(
                      LucideIcons.chevronRight,
                      size: 16,
                      color: context.colors.onSurfaceVariant,
                    ),
                  ],
                ),
              ),
            ),
          ),
      ],
    );
  }
}

class _ChannelsSection extends StatelessWidget {
  final List<Channel> channels;
  final VoidCallback onResultSelected;

  const _ChannelsSection({
    required this.channels,
    required this.onResultSelected,
  });

  @override
  Widget build(BuildContext context) {
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        _SectionLabel(label: 'Channels'),
        for (final channel in channels)
          ListTile(
            key: ValueKey('search-channel-row-${channel.id}'),
            contentPadding: const EdgeInsets.symmetric(horizontal: Grid.gutter),
            leading: Icon(
              channelIcon(channel),
              key: ValueKey('search-channel-leading-${channel.id}'),
              size: 20,
            ),
            title: Text(
              channel.name,
              key: ValueKey('search-channel-title-${channel.id}'),
              style: contentListTitleTextStyle,
            ),
            subtitle: Text(
              '${channel.memberCount} member${channel.memberCount == 1 ? '' : 's'}',
              style: contentListBodyTextStyle.copyWith(
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
            onTap: () {
              onResultSelected();
              Navigator.of(context).push(
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

class _PeopleSection extends ConsumerWidget {
  final List<DirectoryUser> users;
  final VoidCallback onResultSelected;

  const _PeopleSection({required this.users, required this.onResultSelected});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        _SectionLabel(label: 'People'),
        for (final user in users)
          ListTile(
            key: ValueKey('search-person-row-${user.pubkey}'),
            contentPadding: const EdgeInsets.symmetric(horizontal: Grid.gutter),
            leading: AvatarImage(
              key: ValueKey('search-person-leading-${user.pubkey}'),
              imageUrl: user.avatarUrl,
              radius: 20,
              fallback: Text(user.label.substring(0, 1).toUpperCase()),
            ),
            title: Text(
              user.label,
              key: ValueKey('search-person-title-${user.pubkey}'),
              style: contentListTitleTextStyle,
            ),
            subtitle: Text(
              user.secondaryLabel,
              style: contentListBodyTextStyle.copyWith(
                color: context.colors.onSurfaceVariant,
              ),
            ),
            onTap: () async {
              onResultSelected();
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
  final VoidCallback onResultSelected;

  const _MessagesSection({
    required this.hits,
    required this.currentPubkey,
    required this.onResultSelected,
  });

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
            onResultSelected: onResultSelected,
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
  final VoidCallback onResultSelected;

  const _MessageTile({
    required this.hit,
    required this.authorProfile,
    required this.userCache,
    required this.channel,
    required this.currentPubkey,
    required this.onResultSelected,
  });

  @override
  Widget build(BuildContext context) {
    final authorName = authorProfile?.label ?? shortPubkey(hit.pubkey);
    final timeAgo = relativeTime(hit.createdAt);
    final channelName = hit.channelName?.trim().replaceFirst(RegExp(r'^#'), '');
    final hasChannelName = channelName != null && channelName.isNotEmpty;
    final isDm = channel?.isDm ?? false;

    return ListTile(
      key: ValueKey('search-message-row-${hit.eventId}'),
      contentPadding: const EdgeInsets.symmetric(horizontal: Grid.gutter),
      titleAlignment: ListTileTitleAlignment.top,
      horizontalTitleGap: messageAvatarContentGap,
      leading: SmallAvatar(
        key: ValueKey('search-message-avatar-${hit.eventId}'),
        pubkey: hit.pubkey,
        userCache: userCache,
        size: compactMessageAvatarSize,
      ),
      title: MessageAuthorMeta(
        displayName: authorName,
        username: messageUsernameLabel(authorProfile),
        timestamp: timeAgo,
        nameColor: context.colors.onSurface,
        metadataColor: context.colors.onSurfaceVariant,
        displayNameKey: ValueKey('search-message-author-${hit.eventId}'),
        usernameKey: ValueKey('search-message-username-${hit.eventId}'),
        timestampKey: ValueKey('search-message-timestamp-${hit.eventId}'),
      ),
      subtitle: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          const SizedBox(height: 2),
          Row(
            key: ValueKey('search-message-context-${hit.eventId}'),
            children: [
              Flexible(
                child: Text(
                  isDm
                      ? 'Direct message'
                      : hasChannelName
                      ? 'Message in'
                      : 'Message',
                  style: activityContextTextStyle.copyWith(
                    color: context.colors.onSurfaceVariant,
                  ),
                  overflow: TextOverflow.ellipsis,
                ),
              ),
              if (!isDm && hasChannelName) ...[
                const SizedBox(width: Grid.half),
                Flexible(
                  child: Container(
                    padding: const EdgeInsets.symmetric(
                      horizontal: Grid.half + Grid.quarter,
                      vertical: Grid.quarter / 2,
                    ),
                    decoration: BoxDecoration(
                      color: context.colors.surfaceContainerHighest,
                      borderRadius: BorderRadius.circular(Radii.xs),
                    ),
                    child: Text(
                      '#$channelName',
                      key: ValueKey('search-message-channel-${hit.eventId}'),
                      style: activityContextTextStyle.copyWith(
                        color: context.colors.onSurfaceVariant,
                      ),
                      overflow: TextOverflow.ellipsis,
                    ),
                  ),
                ),
              ],
            ],
          ),
          const SizedBox(height: Grid.half),
          MessageContent(
            key: ValueKey('search-message-body-${hit.eventId}'),
            content: hit.content,
            tags: hit.tags,
            maxLines: 2,
            baseStyle: activityPreviewTextStyle.copyWith(
              color: context.colors.onSurface,
            ),
          ),
        ],
      ),
      onTap: () {
        onResultSelected();
        _navigateToHit(context, hit, channel);
      },
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
        label,
        key: ValueKey('search-section-${label.toLowerCase()}'),
        style: activityContextTextStyle.copyWith(
          color: context.colors.onSurfaceVariant,
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
