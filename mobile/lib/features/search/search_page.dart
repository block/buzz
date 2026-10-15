import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:buzz/shared/theme/buzz_icons.dart';
import '../../shared/identity_names/identity_names_provider.dart';
import '../../shared/me/me_search_placement.dart';
import '../../shared/mentions/agent_identity_provider.dart';
import '../../shared/mentions/mention_tags.dart';
import '../../shared/theme/theme.dart';
import '../../shared/widgets/avatar_image.dart';
import '../../shared/widgets/buzz_loading_indicator.dart';
import '../../shared/widgets/buzz_search_field.dart';
import '../../shared/widgets/filter_chip_bar.dart';
import '../../shared/widgets/concentric_sheet_surface.dart';
import '../../shared/widgets/ios_glass_navigation_button.dart';
import '../../shared/widgets/message_author_meta.dart';
import '../channels/channel.dart';
import '../channels/channel_detail_page.dart';
import '../channels/channel_identity_names_provider.dart';
import '../channels/channel_management_provider.dart';
import '../channels/channels_provider.dart';
import '../channels/small_avatar.dart';
import '../channels/message_content.dart';
import '../channels/date_formatters.dart';
import '../forum/forum_thread_page.dart';
import '../profile/profile_provider.dart';
import '../../shared/profile/user_cache_provider.dart';
import '../../shared/profile/user_profile.dart';
import 'recent_searches_provider.dart';
import 'search_provider.dart';

part 'search_page/message_tile.dart';

enum _SearchFilter { all, messages, channels, people }

const _searchIdleFieldHeight = 48.0;
const _searchIdleTextSize = 15.0;
const _searchFieldVerticalPadding = Grid.xxs;
const _searchFilterChipVerticalPadding = Grid.xxs;
const _searchFilterBarVerticalPadding = Grid.xxs;
const _searchHeaderFiltersMinHeight = Grid.xl;

double _idleSearchFieldHeight(BuildContext context) {
  final scaledFontSize = MediaQuery.textScalerOf(
    context,
  ).scale(_searchIdleTextSize);
  final contentHeight =
      scaledFontSize * (20 / _searchIdleTextSize) +
      _searchFieldVerticalPadding * 2;
  return contentHeight > _searchIdleFieldHeight
      ? contentHeight
      : _searchIdleFieldHeight;
}

double _searchHeaderFiltersHeight(BuildContext context) {
  const style = filterChipTextStyle;
  final scaledLabelHeight =
      MediaQuery.textScalerOf(context).scale(style.fontSize ?? 15) *
      (style.height ?? 1);
  final chipHeight = scaledLabelHeight + _searchFilterChipVerticalPadding * 2;
  final contentHeight = chipHeight + _searchFilterBarVerticalPadding * 2;
  return contentHeight > _searchHeaderFiltersMinHeight
      ? contentHeight
      : _searchHeaderFiltersMinHeight;
}

class SearchPage extends HookConsumerWidget {
  const SearchPage({this.tabReselection, super.key});

  /// Notifies this page when its already-selected tab is tapped again.
  final ValueListenable<int>? tabReselection;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final searchState = ref.watch(searchProvider);
    final currentPubkey = ref
        .watch(profileProvider)
        .whenData((value) => value?.pubkey)
        .value;
    final activeFilter = useState(_SearchFilter.all);
    final textController = useTextEditingController(text: searchState.query);
    final focusNode = useFocusNode();
    final nativeIos = defaultTargetPlatform == TargetPlatform.iOS;
    final searchSurfaceColor = navigationSearchSurface(context);
    final searchPrimaryColor = navigationPrimaryForeground(context);
    final searchPlaceholderColor = navigationSecondaryForeground(context);
    final fieldHeight = _idleSearchFieldHeight(context);
    final filtersHeight = nativeIos
        ? (MediaQuery.textScalerOf(context).scale(16) * 1.4 + 16).clamp(
                48.0,
                double.infinity,
              ) +
              Grid.xxs * 2
        : _searchHeaderFiltersHeight(context);

    void activateSearch() => focusNode.requestFocus();

    useEffect(() {
      final reselection = tabReselection;
      if (reselection == null) return null;
      void reactivateSearch() {
        focusNode.requestFocus();
        WidgetsBinding.instance.addPostFrameCallback((_) {
          if (context.mounted) focusNode.requestFocus();
        });
        WidgetsBinding.instance.ensureVisualUpdate();
      }

      reselection.addListener(reactivateSearch);
      return () => reselection.removeListener(reactivateSearch);
    }, [tabReselection, focusNode]);

    void closeSearch() {
      focusNode.unfocus();
      Navigator.of(context).maybePop();
    }

    void runRecentSearch(String query) {
      textController.value = TextEditingValue(
        text: query,
        selection: TextSelection.collapsed(offset: query.length),
      );
      activateSearch();
      ref.read(recentSearchesProvider.notifier).record(query);
      ref.read(searchProvider.notifier).search(query);
    }

    return Scaffold(
      backgroundColor: context.colors.surface,
      resizeToAvoidBottomInset: false,
      body: SafeArea(
        bottom: false,
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Padding(
              padding: const EdgeInsets.fromLTRB(
                Grid.gutter,
                Grid.half,
                Grid.gutter,
                Grid.xxs,
              ),
              child: Row(
                children: [
                  Expanded(
                    child: SizedBox(
                      key: const Key('search-field-container'),
                      height: fieldHeight,
                      child: ConcentricSheetSurface(
                        enabled: nativeIos,
                        usesGlass: true,
                        providesSheetSurface: false,
                        padding: EdgeInsets.zero,
                        color: searchSurfaceColor,
                        minimumRadius: fieldHeight / 2,
                        contentClipRadius: fieldHeight / 2,
                        child: BuzzSearchField(
                          controller: textController,
                          focusNode: focusNode,
                          hintText: 'Search messages, channels, and people',
                          iconColor: searchPrimaryColor,
                          inputColor: searchPrimaryColor,
                          placeholderColor: searchPlaceholderColor,
                          surfaceColor: nativeIos
                              ? Colors.transparent
                              : searchSurfaceColor,
                          isEditing: false,
                          reduceMotion: true,
                          motionDuration: Duration.zero,
                          onTap: activateSearch,
                          onChanged: (value) =>
                              ref.read(searchProvider.notifier).search(value),
                          onSubmitted: (value) {
                            final query = value.trim();
                            if (query.isNotEmpty) {
                              ref
                                  .read(recentSearchesProvider.notifier)
                                  .record(query);
                            }
                          },
                        ),
                      ),
                    ),
                  ),
                  const SizedBox(width: Grid.xxs),
                  if (nativeIos)
                    IosGlassNavigationButton(
                      key: const Key('search-close'),
                      icon: IosGlassNavigationIcon.close,
                      semanticLabel: 'Close search',
                      onPressed: closeSearch,
                      controlSize: 48,
                      foregroundColor: searchPrimaryColor,
                    )
                  else
                    SizedBox.square(
                      key: const Key('search-close'),
                      dimension: 48,
                      child: IconButton(
                        tooltip: 'Close search',
                        onPressed: closeSearch,
                        style: IconButton.styleFrom(
                          backgroundColor: searchSurfaceColor,
                          foregroundColor: searchPrimaryColor,
                        ),
                        icon: const Icon(BuzzIcons.x),
                      ),
                    ),
                ],
              ),
            ),
            SizedBox(
              key: const ValueKey('search-header-filters'),
              height: filtersHeight,
              child: FilterChipBar<_SearchFilter>(
                useNativeGlass: nativeIos,
                expandItems: true,
                visualDensity: const VisualDensity(horizontal: -2),
                chipVerticalPadding: _searchFilterChipVerticalPadding,
                barVerticalPadding: _searchFilterBarVerticalPadding,
                selected: activeFilter.value,
                onSelected: (filter) => activeFilter.value = filter,
                items: [
                  for (final filter in _SearchFilter.values)
                    FilterChipItem(id: filter, label: filter.label),
                ],
              ),
            ),
            const SizedBox(height: Grid.xxs),
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
                BuzzIcons.search,
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
        bottom:
            Grid.xl +
            MediaQuery.paddingOf(context).bottom +
            MediaQuery.viewInsetsOf(context).bottom,
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
            child: Center(
              child: BuzzLoadingIndicator(
                size: 36,
                semanticLabel: 'Loading more search results',
              ),
            ),
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
        bottom:
            Grid.xl +
            MediaQuery.paddingOf(context).bottom +
            MediaQuery.viewInsetsOf(context).bottom,
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
                      BuzzIcons.clock,
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
                      BuzzIcons.chevronRight,
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

class _ChannelsSection extends ConsumerWidget {
  final List<Channel> channels;
  final VoidCallback onResultSelected;
  const _ChannelsSection({
    required this.channels,
    required this.onResultSelected,
  });
  @override
  Widget build(BuildContext context, WidgetRef ref) {
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
            subtitle: channel.isMember
                ? Text(
                    '${channel.memberCount} member${channel.memberCount == 1 ? '' : 's'}',
                    style: contentListBodyTextStyle.copyWith(
                      color: context.colors.onSurfaceVariant,
                    ),
                  )
                : null,
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
              _openSearchChannel(context, ref, channel);
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
    // The shown results are the comparison context.
    final names = watchIdentityNames(
      ref,
      [for (final user in users) user.pubkey],
      agentPubkeys: {
        for (final user in users)
          if (user.isAgent) user.pubkey,
      },
      fallbackNames: {for (final user in users) user.pubkey: user.label},
    );
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
              fallback: Text(user.initial),
              isAgent: user.isAgent,
            ),
            title: Text(
              names.labelFor(user.pubkey),
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

class _MessagesSection extends HookConsumerWidget {
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
    final preloadPubkeys = {
      for (final hit in hits) hit.pubkey.toLowerCase(),
      for (final hit in hits) ...mentionedPubkeysFromTags(hit.tags),
    }.toList()..sort();
    final preloadPubkeysKey = preloadPubkeys.join('\u0000');
    useEffect(() {
      ref.read(userCacheProvider.notifier).preload(preloadPubkeys);
      return null;
    }, [preloadPubkeysKey]);

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

extension on _SearchFilter {
  String get label => switch (this) {
    _SearchFilter.all => 'All',
    _SearchFilter.messages => 'Messages',
    _SearchFilter.channels => 'Channels',
    _SearchFilter.people => 'People',
  };
}

/// Resolve personal placement before opening a result so sessions retain their
/// flat conversation and assistant controls, including on a fresh app launch.
Future<void> _openSearchChannel(
  BuildContext context,
  WidgetRef ref,
  Channel channel,
) async {
  try {
    final personal = await ref.read(
      meSearchPlacementProvider(channel.id).future,
    );
    if (!context.mounted) return;
    await Navigator.of(context).push(
      MaterialPageRoute<void>(
        builder: (_) =>
            ChannelDetailPage(channel: channel, personalChat: personal),
      ),
    );
  } catch (_) {
    if (!context.mounted) return;
    ScaffoldMessenger.of(context).showSnackBar(
      const SnackBar(
        content: Text('Couldn’t open this conversation. Try again.'),
      ),
    );
  }
}
