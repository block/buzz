import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';
import '../../shared/mentions/agent_identity_provider.dart';
import '../../shared/mentions/mention_tags.dart';
import '../../shared/theme/theme.dart';
import '../../shared/widgets/avatar_image.dart';
import '../../shared/widgets/buzz_loading_indicator.dart';
import '../../shared/widgets/buzz_search_field.dart';
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
import '../../shared/profile/user_cache_provider.dart';
import '../../shared/profile/user_profile.dart';
import 'recent_searches_provider.dart';
import 'search_provider.dart';

enum _SearchFilter { all, messages, channels, people }

const _searchFieldMinHeight = 36.0;
const _searchIdleFieldHeight = 45.0;
const _searchIdleTextSize = 15.0;
const _searchFieldVerticalPadding = Grid.xxs;
const _searchFieldMoveDuration = Duration(milliseconds: 160);
const _searchTitleReturnDuration = Duration(milliseconds: 80);
const _searchCancelEnterDuration = Duration(milliseconds: 80);
const _searchCancelExitDuration = Duration(milliseconds: 60);
const _searchIdleFieldTopInset = Grid.half;
const _searchControlsToFiltersGap = Grid.xxs;
const _searchFilterChipVerticalPadding = Grid.xxs;
const _searchFilterBarVerticalPadding = Grid.xxs;
const _searchHeaderFiltersMinHeight = Grid.xl;
const _searchActiveFieldRightInsetMin = 72.0;

/// Reserves the Cancel action's scaled label, padding, and app-bar edge inset.
double _searchActiveFieldRightInset(BuildContext context) {
  final textPainter = TextPainter(
    text: TextSpan(
      text: 'Cancel',
      style: filterChipTextStyle.copyWith(fontWeight: FontWeight.w500),
    ),
    textScaler: MediaQuery.textScalerOf(context),
    textDirection: Directionality.of(context),
  )..layout();
  final cancelWidth = textPainter.width + Grid.half * 2 + Grid.twelve;
  return cancelWidth > _searchActiveFieldRightInsetMin
      ? cancelWidth
      : _searchActiveFieldRightInsetMin;
}

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
    final textController = useTextEditingController();
    final focusNode = useFocusNode();
    final isSearchEditing = useState(false);
    final showSearchTitle = useState(true);
    final isTabActivationInFlight = useRef(false);
    final reduceMotion = MediaQuery.disableAnimationsOf(context);
    final searchSurfaceColor = navigationSearchSurface(context);
    final searchPrimaryColor = navigationPrimaryForeground(context);
    final searchPlaceholderColor = navigationSecondaryForeground(context);
    final headerTitleStyle = context.textTheme.titleMedium?.copyWith(
      fontSize: 22,
      fontWeight: FontWeight.w600,
    );
    final compactSearchFieldHeight = _searchFieldHeight(context);
    final idleSearchFieldHeight = _idleSearchFieldHeight(context);
    final searchHeaderFiltersHeight = _searchHeaderFiltersHeight(context);
    final searchActiveFieldRightInset = _searchActiveFieldRightInset(context);
    final searchBottomOverlap =
        _searchIdleFieldTopInset +
        compactSearchFieldHeight +
        _searchControlsToFiltersGap;
    // Cancel remains an accessible target without giving the text action a
    // visual button treatment.
    final searchControlHeight = compactSearchFieldHeight > Grid.xl
        ? compactSearchFieldHeight
        : Grid.xl;
    final searchHeaderBottomHeight = isSearchEditing.value
        ? searchHeaderFiltersHeight + _searchControlsToFiltersGap
        : idleSearchFieldHeight + _searchIdleFieldTopInset + Grid.xxs;
    final topSectionHeight = frostedAppBarHeight(
      context,
      titleStyle: headerTitleStyle,
      bottomHeight: searchHeaderBottomHeight,
    );

    void activateSearch() {
      showSearchTitle.value = false;
      isSearchEditing.value = true;
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (context.mounted && isSearchEditing.value) {
          focusNode.requestFocus();
        }
      });
    }

    void deactivateSearch() {
      if (!isSearchEditing.value) return;
      // The field is painted above the title while it returns to its idle
      // position. Start this fade now so the title is already there beneath it
      // and is revealed by the field's motion instead of arriving afterward.
      showSearchTitle.value = true;
      isSearchEditing.value = false;
    }

    useEffect(() {
      final tabReselection = this.tabReselection;
      if (tabReselection == null) return null;

      void reactivateSearch() {
        // The same tab gesture can report focus loss after this callback. Keep
        // that notification from starting a competing return animation while
        // the normal field activation path restores focus.
        isTabActivationInFlight.value = true;
        activateSearch();
        WidgetsBinding.instance.addPostFrameCallback((_) {
          isTabActivationInFlight.value = false;
        });
      }

      tabReselection.addListener(reactivateSearch);
      return () => tabReselection.removeListener(reactivateSearch);
    }, [tabReselection, focusNode]);

    useEffect(() {
      void resetIdlePromptWhenFocusLeaves() {
        if (!focusNode.hasFocus && !isTabActivationInFlight.value) {
          deactivateSearch();
        }
      }

      focusNode.addListener(resetIdlePromptWhenFocusLeaves);
      return () => focusNode.removeListener(resetIdlePromptWhenFocusLeaves);
    }, [focusNode]);

    void runRecentSearch(String query) {
      textController.value = TextEditingValue(
        text: query,
        selection: TextSelection.collapsed(offset: query.length),
      );
      activateSearch();
      ref.read(recentSearchesProvider.notifier).record(query);
      ref.read(searchProvider.notifier).search(query);
    }

    return FrostedScaffold(
      backgroundColor: context.colors.surface,
      // Keep the empty state centered in the page rather than the portion left
      // above the keyboard.
      resizeToAvoidBottomInset: false,
      appBar: FrostedAppBar(
        automaticallyImplyLeading: false,
        horizontalInset: Grid.twelve,
        showBottomDivider: true,
        bottomDividerOpacity: 0.07,
        titleStyle: headerTitleStyle,
        // Keep this mounted through the search-field morph so it can fade in
        // beneath the returning field rather than popping in afterward.
        title: IgnorePointer(
          child: AnimatedOpacity(
            key: const Key('search-header-title-opacity'),
            duration: reduceMotion ? Duration.zero : _searchTitleReturnDuration,
            curve: Curves.easeOutCubic,
            opacity: showSearchTitle.value ? 1 : 0,
            child: Text(
              'Search',
              key: const ValueKey('search-header-title'),
              style: headerTitleStyle,
            ),
          ),
        ),
        actions: [
          AnimatedSwitcher(
            duration: reduceMotion ? Duration.zero : _searchCancelEnterDuration,
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
            child: isSearchEditing.value
                ? Semantics(
                    key: const Key('search-cancel'),
                    button: true,
                    label: 'Cancel search',
                    child: GestureDetector(
                      behavior: HitTestBehavior.opaque,
                      onTap: () {
                        textController.clear();
                        ref.read(searchProvider.notifier).clear();
                        deactivateSearch();
                        focusNode.unfocus();
                      },
                      child: SizedBox(
                        height: searchControlHeight,
                        child: Padding(
                          padding: const EdgeInsets.symmetric(
                            horizontal: Grid.half,
                          ),
                          child: Center(
                            child: Text(
                              'Cancel',
                              style: filterChipTextStyle.copyWith(
                                color: context.colors.primary,
                                fontWeight: FontWeight.w500,
                              ),
                            ),
                          ),
                        ),
                      ),
                    ),
                  )
                : const SizedBox.shrink(key: ValueKey('search-cancel-hidden')),
          ),
        ],
        bottomHeight: searchHeaderBottomHeight,
        bottomOverlap: searchBottomOverlap,
        bottom: Stack(
          clipBehavior: Clip.none,
          children: [
            AnimatedPositioned(
              duration: reduceMotion ? Duration.zero : _searchFieldMoveDuration,
              curve: Curves.easeInOutCubic,
              left: Grid.gutter,
              right: isSearchEditing.value
                  ? searchActiveFieldRightInset
                  : Grid.gutter,
              top: isSearchEditing.value
                  ? _searchIdleFieldTopInset
                  : searchBottomOverlap + _searchIdleFieldTopInset,
              height: isSearchEditing.value
                  ? compactSearchFieldHeight
                  : idleSearchFieldHeight,
              // Do not key this subtree by the visual state: replacing the
              // TextField immediately after its first tap can detach its
              // native input connection before the keyboard is shown.
              child: SizedBox(
                key: const Key('search-field-container'),
                child: BuzzSearchField(
                  controller: textController,
                  focusNode: focusNode,
                  hintText: 'Search messages, channels, and people',
                  iconColor: searchPrimaryColor,
                  inputColor: searchPrimaryColor,
                  placeholderColor: searchPlaceholderColor,
                  surfaceColor: searchSurfaceColor,
                  isEditing: isSearchEditing.value,
                  reduceMotion: reduceMotion,
                  motionDuration: _searchFieldMoveDuration,
                  onTap: activateSearch,
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
            Positioned.fill(
              child: AnimatedSwitcher(
                duration: reduceMotion
                    ? Duration.zero
                    : _searchCancelEnterDuration,
                switchInCurve: Curves.easeOutCubic,
                switchOutCurve: Curves.easeInCubic,
                transitionBuilder: (child, animation) => FadeTransition(
                  opacity: animation,
                  child: SlideTransition(
                    position: Tween<Offset>(
                      begin: const Offset(0, 0.2),
                      end: Offset.zero,
                    ).animate(animation),
                    child: child,
                  ),
                ),
                child: isSearchEditing.value
                    ? Align(
                        alignment: Alignment.topCenter,
                        child: Padding(
                          padding: EdgeInsets.only(top: searchBottomOverlap),
                          child: SizedBox(
                            key: const ValueKey('search-header-filters'),
                            height: searchHeaderFiltersHeight,
                            child: FilterChipBar<_SearchFilter>(
                              expandItems: true,
                              visualDensity: const VisualDensity(
                                horizontal: -2,
                              ),
                              chipVerticalPadding:
                                  _searchFilterChipVerticalPadding,
                              barVerticalPadding:
                                  _searchFilterBarVerticalPadding,
                              selected: activeFilter.value,
                              onSelected: (f) => activeFilter.value = f,
                              items: [
                                for (final f in _SearchFilter.values)
                                  FilterChipItem(id: f, label: f.label),
                              ],
                            ),
                          ),
                        ),
                      )
                    : const SizedBox.shrink(
                        key: ValueKey('search-header-filters-hidden'),
                      ),
              ),
            ),
          ],
        ),
      ),
      body: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          SizedBox(height: topSectionHeight),
          Expanded(
            child: ClipRRect(
              borderRadius: const BorderRadius.vertical(
                top: Radius.circular(Radii.dialog),
              ),
              child: ColoredBox(
                color: context.colors.surface,
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.stretch,
                  children: [
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
              isAgent: user.isAgent,
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

class _MessageTile extends ConsumerWidget {
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
  Widget build(BuildContext context, WidgetRef ref) {
    final authorName = authorProfile?.label ?? shortPubkey(hit.pubkey);
    final timeAgo = relativeTime(hit.createdAt);
    final channelName = hit.channelName?.trim().replaceFirst(RegExp(r'^#'), '');
    final hasChannelName = channelName != null && channelName.isNotEmpty;
    final isDm = channel?.isDm ?? false;
    final profileMentionNames = {
      for (final pubkey in mentionedPubkeysFromTags(hit.tags))
        if (userCache[pubkey]?.displayName?.trim().isNotEmpty == true)
          pubkey: userCache[pubkey]!.displayName!.trim(),
    };
    final mentionPubkeys = mentionedPubkeysFromTags(hit.tags);
    final knownAgentPubkeys = channel == null
        ? ref.watch(knownAgentPubkeysProvider)
        : ref.watch(agentMentionPubkeysProvider(channel!.id));
    final agentMentionPubkeys = agentPubkeysWithProfileOwners(
      knownAgentPubkeys: knownAgentPubkeys,
      profileOwnedAgentPubkeys: [
        for (final profile in userCache.values)
          if (profile.ownerPubkey != null) profile.pubkey,
      ],
    );
    final mentionNames = mentionNamesWithDirectoryLabels(
      mentionPubkeys: mentionPubkeys,
      profileMentionNames: profileMentionNames,
      directoryDisplayNames: ref.watch(agentDirectoryDisplayNamesProvider),
      agentMentionPubkeys: agentMentionPubkeys,
    );

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
            mentionNames: mentionNames,
            agentMentionPubkeys: agentMentionPubkeys,
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
