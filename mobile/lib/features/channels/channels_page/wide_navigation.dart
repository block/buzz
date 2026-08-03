part of '../channels_page.dart';

const _wideNavigationWidth = 280.0;
const _wideNavigationIconSize = 20.0;
const _wideNavigationDmAvatarSize = 24.0;
const _wideNavigationPrimaryRowHeight = 52.0;
const _wideNavigationChannelRowHeight = 48.0;
const _wideNavigationLabelGap = Grid.twelve - Grid.quarter;
const _wideNavigationDmLabelGap = _wideNavigationLabelGap - Grid.quarter;
const _wideNavigationIdentityAvatarInset =
    Grid.xs + Grid.half - (Grid.quarter / 2);
const _wideNavigationIdentityLabelGap = Grid.half + (Grid.quarter / 2);

/// A top-level tablet workspace destination supplied by the Home shell.
class WideNavigationDestination {
  final IconData icon;
  final IconData selectedIcon;
  final String label;

  const WideNavigationDestination({
    required this.icon,
    required this.selectedIcon,
    required this.label,
  });
}

/// Tablet channel sidebar that owns channel/profile presentation and callbacks.
class WideChannelsNavigation extends HookConsumerWidget {
  final int? selectedIndex;
  final ValueChanged<int> onDestinationSelected;
  final ValueChanged<String> onChannelSelected;
  final String? selectedChannelId;
  final VoidCallback onProfileSelected;
  final bool isCommunitySwitching;
  final ValueChanged<String?> onCommunitySwitchStart;
  final String? pendingCommunityId;
  final bool isActivitySettled;
  final VoidCallback onCommunitySwitchComplete;
  final List<WideNavigationDestination> destinations;

  const WideChannelsNavigation({
    super.key,
    required this.selectedIndex,
    required this.onDestinationSelected,
    required this.onChannelSelected,
    required this.selectedChannelId,
    required this.onProfileSelected,
    required this.isCommunitySwitching,
    required this.onCommunitySwitchStart,
    required this.pendingCommunityId,
    required this.isActivitySettled,
    required this.onCommunitySwitchComplete,
    required this.destinations,
  });

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final colors = context.colors;
    final sidebarGradient = context.appColors.topSectionGradient;
    final sidebarPalette = _wideSidebarPalette(context);
    // The workspace sidebar only needs the signed-in profile identity to
    // label direct-message participants. Keep that navigation concern out of
    // the channel-management implementation.
    final currentPubkey = ref.watch(profileProvider).value?.pubkey;
    final profiles = ref.watch(userCacheProvider);
    final channelsAsync = ref.watch(channelsProvider);
    final activeCommunityId = ref.watch(activeCommunityProvider).value?.id;
    useEffect(
      () {
        if (pendingCommunityId != activeCommunityId ||
            channelsAsync.isLoading ||
            !(channelsAsync.hasValue || channelsAsync.hasError) ||
            !isActivitySettled) {
          return null;
        }
        final timer = Timer(
          const Duration(milliseconds: 250),
          onCommunitySwitchComplete,
        );
        return timer.cancel;
      },
      [
        pendingCommunityId,
        activeCommunityId,
        channelsAsync.isLoading,
        channelsAsync.hasValue,
        channelsAsync.hasError,
        isActivitySettled,
      ],
    );
    final channels = (channelsAsync.asData?.value ?? const <Channel>[])
        .where((channel) => !channel.isArchived)
        .toList();
    final streamChannels = channels.where((channel) => !channel.isDm).toList();
    final directMessages = channels.where((channel) => channel.isDm).toList();
    streamChannels.sort(
      (left, right) => left.displayLabel().compareTo(right.displayLabel()),
    );
    directMessages.sort(
      (left, right) =>
          _wideSidebarChannelLabel(
            left,
            currentPubkey: currentPubkey,
            profiles: profiles,
          ).compareTo(
            _wideSidebarChannelLabel(
              right,
              currentPubkey: currentPubkey,
              profiles: profiles,
            ),
          ),
    );
    final participantPubkeys = [
      for (final channel in directMessages)
        for (final pubkey in channel.participantPubkeys)
          if (pubkey.toLowerCase() != currentPubkey?.toLowerCase())
            pubkey.toLowerCase(),
    ];
    final participantPubkeysKey = participantPubkeys.toSet().join('\u0000');
    useEffect(() {
      ref.read(userCacheProvider.notifier).preload(participantPubkeys);
      return null;
    }, [participantPubkeysKey]);

    Future<void> createChannel() async {
      final channel = await showCreateChannelSheet(context);
      if (channel != null) onChannelSelected(channel.id);
    }

    Future<void> createDirectMessage() async {
      final channel = await showNewDirectMessageSheet(
        context,
        currentPubkey: currentPubkey,
      );
      if (channel != null) onChannelSelected(channel.id);
    }

    return SizedBox(
      key: const Key('wide-navigation-sidebar'),
      width: _wideNavigationWidth,
      child: DecoratedBox(
        key: const Key('wide-navigation-sidebar-background'),
        decoration: BoxDecoration(
          color: sidebarGradient == null ? colors.surfaceContainerLowest : null,
          gradient: sidebarGradient,
        ),
        child: SafeArea(
          right: false,
          bottom: false,
          child: Padding(
            padding: const EdgeInsets.only(left: Grid.xxs, right: Grid.xxs),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                _WideNavigationCommunitySwitcher(
                  onTap: () => unawaited(
                    showCommunitySwitcherSheet(
                      context,
                      onCommunitySwitchStart: onCommunitySwitchStart,
                    ),
                  ),
                  onCommunitySwitchStart: onCommunitySwitchStart,
                ),
                const SizedBox(height: Grid.xxs),
                Expanded(
                  child: SkeletonReveal(
                    loading: isCommunitySwitching,
                    skeleton: const SizedBox(
                      key: Key('wide-navigation-community-switch-skeleton'),
                    ),
                    content: Column(
                      crossAxisAlignment: CrossAxisAlignment.stretch,
                      children: [
                        // The desktop sidebar presents Search before Inbox.
                        // Keep these source indexes so the phone tab order and
                        // each destination's route stay unchanged.
                        for (final index in const [2, 1])
                          _WideNavigationDestination(
                            destination: destinations[index],
                            selected: index == selectedIndex,
                            onTap: () => onDestinationSelected(index),
                          ),
                        const SizedBox(height: Grid.xxs),
                        Expanded(
                          child: ListView(
                            key: const Key('wide-navigation-channel-list'),
                            padding: EdgeInsets.zero,
                            children: [
                              _WideNavigationSectionLabel(
                                'Channels',
                                actionTooltip: 'Create channel',
                                onAction: () => unawaited(createChannel()),
                              ),
                              if (channelsAsync.isLoading)
                                Padding(
                                  padding: EdgeInsets.all(Grid.sm),
                                  child: Center(
                                    child: SizedBox(
                                      width: Grid.md,
                                      height: Grid.md,
                                      child: CircularProgressIndicator(
                                        strokeWidth: 2,
                                        color: sidebarPalette.mutedForeground,
                                      ),
                                    ),
                                  ),
                                ),
                              for (final channel in streamChannels)
                                _WideChannelDestination(
                                  channel: channel,
                                  label: _wideSidebarChannelLabel(
                                    channel,
                                    currentPubkey: currentPubkey,
                                    profiles: profiles,
                                  ),
                                  selected: channel.id == selectedChannelId,
                                  onTap: () => onChannelSelected(channel.id),
                                ),
                              const SizedBox(height: Grid.xs),
                              _WideNavigationSectionLabel(
                                'DMs',
                                actionTooltip: 'New direct message',
                                onAction: () =>
                                    unawaited(createDirectMessage()),
                              ),
                              for (final channel in directMessages)
                                _WideChannelDestination(
                                  channel: channel,
                                  label: _wideSidebarChannelLabel(
                                    channel,
                                    currentPubkey: currentPubkey,
                                    profiles: profiles,
                                  ),
                                  directMessageProfile:
                                      _wideSidebarDirectMessageProfile(
                                        channel,
                                        currentPubkey: currentPubkey,
                                        profiles: profiles,
                                      ),
                                  selected: channel.id == selectedChannelId,
                                  onTap: () => onChannelSelected(channel.id),
                                ),
                            ],
                          ),
                        ),
                        const SizedBox(height: Grid.xxs),
                        Padding(
                          padding: const EdgeInsets.only(bottom: Grid.xs),
                          child: _WideNavigationProfile(
                            onTap: onProfileSelected,
                          ),
                        ),
                      ],
                    ),
                  ),
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}

class _WideSidebarPalette {
  final Color foreground;
  final Color mutedForeground;
  final Color activeForeground;
  final Color activeSurface;

  const _WideSidebarPalette({
    required this.foreground,
    required this.mutedForeground,
    required this.activeForeground,
    required this.activeSurface,
  });
}

_WideSidebarPalette _wideSidebarPalette(BuildContext context) {
  final colors = context.colors;
  if (context.appColors.topSectionGradient == null) {
    return _WideSidebarPalette(
      foreground: colors.onSurfaceVariant,
      mutedForeground: colors.onSurfaceVariant,
      activeForeground: colors.primary,
      activeSurface: colors.primary.withValues(alpha: 0.12),
    );
  }

  final foreground = colors.onSurface;
  return _WideSidebarPalette(
    foreground: foreground,
    mutedForeground: foreground.withValues(alpha: 0.4),
    activeForeground: foreground,
    activeSurface: foreground.withValues(
      alpha: colors.brightness == Brightness.dark ? 0.16 : 0.07,
    ),
  );
}

/// Element-shaped loading state for a community switch. It mirrors the wide
/// navigation's normal hierarchy so a relay change reads as a new workspace,
/// rather than leaving the previous community's rows in place.
class _WideNavigationSectionLabel extends StatelessWidget {
  final String label;
  final String actionTooltip;
  final VoidCallback onAction;

  const _WideNavigationSectionLabel(
    this.label, {
    required this.actionTooltip,
    required this.onAction,
  });

  @override
  Widget build(BuildContext context) {
    final palette = _wideSidebarPalette(context);
    return SizedBox(
      height: 44,
      child: Padding(
        padding: const EdgeInsets.only(left: Grid.sm),
        child: Row(
          children: [
            Expanded(
              child: Text(
                label,
                style: context.textTheme.labelMedium?.copyWith(
                  color: palette.mutedForeground,
                  fontWeight: FontWeight.w600,
                ),
              ),
            ),
            IconButton(
              onPressed: onAction,
              tooltip: actionTooltip,
              color: palette.mutedForeground,
              icon: const Icon(LucideIcons.plus, size: 20),
            ),
          ],
        ),
      ),
    );
  }
}

class _WideNavigationCommunitySwitcher extends HookConsumerWidget {
  final VoidCallback onTap;
  final ValueChanged<String?> onCommunitySwitchStart;

  const _WideNavigationCommunitySwitcher({
    required this.onTap,
    required this.onCommunitySwitchStart,
  });

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final palette = _wideSidebarPalette(context);
    final community = ref.watch(activeCommunityProvider).value;
    final communities =
        ref.watch(communityListProvider).value ?? const <Community>[];
    final activeIndex = communities.indexWhere(
      (candidate) => candidate.id == community?.id,
    );
    final calculatedNextCommunity = communities.length < 2
        ? null
        : communities[activeIndex < 0
              ? 0
              : (activeIndex + 1) % communities.length];
    // Keep the selected destination stable while the provider changes the
    // active community. Without this, the incoming slot can briefly be
    // recalculated as the *following* community during the handoff.
    final switchTarget = useState<Community?>(null);
    final nextCommunity = switchTarget.value ?? calculatedNextCommunity;
    final displayedCommunity = switchTarget.value ?? community;
    final communityName = displayedCommunity?.name ?? 'Community';
    final relayUrl = community?.relayUrl;
    final iconUrl = relayUrl == null
        ? null
        : ref.watch(communityIconProvider(relayUrl)).value;
    final nextRelayUrl = nextCommunity?.relayUrl;
    final nextIconUrl = nextRelayUrl == null
        ? null
        : ref.watch(communityIconProvider(nextRelayUrl)).value;
    final reduceMotion = MediaQuery.disableAnimationsOf(context);
    final swipeProgress = useAnimationController(
      duration: const Duration(milliseconds: 120),
      reverseDuration: const Duration(milliseconds: 160),
    );
    final swipeValue = useAnimation(swipeProgress);
    final isSwitching = useState(false);

    Future<void> switchToNextCommunity() async {
      if (isSwitching.value || nextCommunity == null) return;
      isSwitching.value = true;
      switchTarget.value = nextCommunity;

      if (reduceMotion) {
        swipeProgress.value = 1;
      } else {
        await swipeProgress.animateTo(1, curve: Curves.easeOutCubic);
      }

      // Let the incoming community settle for a painted frame before the
      // workspace skeleton takes over; otherwise its completed arrival is
      // visually skipped by the loading transition.
      if (!reduceMotion) {
        await Future<void>.delayed(const Duration(milliseconds: 120));
      }

      if (!context.mounted) return;
      onCommunitySwitchStart(nextCommunity.id);
      unawaited(HapticFeedback.selectionClick());
      try {
        await ref
            .read(communityListProvider.notifier)
            .switchCommunity(nextCommunity.id);
      } catch (error) {
        debugPrint('[CommunitySwitcher] failed to switch community: $error');
        if (!context.mounted) return;
        onCommunitySwitchStart(null);
        swipeProgress.value = 0;
        switchTarget.value = null;
        isSwitching.value = false;
        return;
      }
      if (!context.mounted) return;
      swipeProgress.value = 0;
      switchTarget.value = null;
      isSwitching.value = false;
    }

    void resetSwipe() {
      if (isSwitching.value) return;
      if (reduceMotion) {
        swipeProgress.value = 0;
      } else {
        unawaited(swipeProgress.reverse());
      }
    }

    return Semantics(
      key: const Key('wide-navigation-community-switcher'),
      button: true,
      label: 'Switch community from $communityName',
      child: SizedBox(
        height: 48,
        child: Material(
          color: Colors.transparent,
          borderRadius: BorderRadius.circular(Radii.md),
          clipBehavior: Clip.antiAlias,
          child: InkWell(
            onTap: onTap,
            child: Align(
              alignment: Alignment.centerLeft,
              child: Padding(
                padding: const EdgeInsets.only(
                  left: _wideNavigationIdentityAvatarInset,
                  right: Grid.sm,
                ),
                child: Row(
                  key: const Key('wide-navigation-community-identity'),
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    _WideNavigationCommunityIcon(
                      iconUrl: iconUrl,
                      nextIconUrl: nextIconUrl,
                      hasNextCommunity: nextCommunity != null,
                      dragProgress: reduceMotion ? 0 : swipeValue,
                      onVerticalDragStart: () => swipeProgress.stop(),
                      onVerticalDragUpdate: (delta) {
                        if (isSwitching.value) return;
                        swipeProgress.value =
                            (swipeProgress.value + (delta / 80)).clamp(0, 1);
                      },
                      onVerticalDragEnd: (velocity) {
                        final shouldSwitch =
                            communities.length > 1 &&
                            (swipeProgress.value >= 0.45 || velocity > 600);
                        if (shouldSwitch) {
                          unawaited(switchToNextCommunity());
                        } else {
                          resetSwipe();
                        }
                      },
                      onVerticalDragCancel: resetSwipe,
                    ),
                    const SizedBox(width: _wideNavigationIdentityLabelGap),
                    ConstrainedBox(
                      constraints: const BoxConstraints(maxWidth: 192),
                      // The avatar motion carries the change. Keeping the
                      // label in a fixed slot avoids a competing crossfade and
                      // relayout while the user is dragging.
                      child: Text(
                        communityName,
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                        style: context.textTheme.bodyMedium?.copyWith(
                          color: palette.foreground,
                          fontSize: 18,
                          fontWeight: FontWeight.w600,
                        ),
                      ),
                    ),
                  ],
                ),
              ),
            ),
          ),
        ),
      ),
    );
  }
}

class _WideNavigationCommunityIcon extends StatelessWidget {
  final String? iconUrl;
  final String? nextIconUrl;
  final bool hasNextCommunity;
  final double dragProgress;
  final VoidCallback onVerticalDragStart;
  final ValueChanged<double> onVerticalDragUpdate;
  final ValueChanged<double> onVerticalDragEnd;
  final VoidCallback onVerticalDragCancel;

  const _WideNavigationCommunityIcon({
    required this.iconUrl,
    required this.nextIconUrl,
    required this.hasNextCommunity,
    required this.dragProgress,
    required this.onVerticalDragStart,
    required this.onVerticalDragUpdate,
    required this.onVerticalDragEnd,
    required this.onVerticalDragCancel,
  });

  @override
  Widget build(BuildContext context) {
    // A completed first phase leaves no visual overlap: the current community
    // exits before its replacement starts to appear.
    final outgoingProgress = dragProgress < 0.5 ? dragProgress * 2 : 1.0;
    final incomingProgress = dragProgress > 0.5
        ? (dragProgress - 0.5) * 2
        : 0.0;

    Widget iconImage(String? imageUrl, Key key) {
      return ClipRRect(
        key: key,
        borderRadius: BorderRadius.circular(Radii.card),
        child: ColoredBox(
          color: context.colors.primaryContainer,
          child: AvatarImageContent(
            imageUrl: imageUrl,
            fallback: Text(
              '🐝',
              textScaler: TextScaler.noScaling,
              style: const TextStyle(fontSize: 18, height: 1),
            ),
          ),
        ),
      );
    }

    return SizedBox.square(
      key: const Key('wide-navigation-community-icon'),
      dimension: 32,
      child: GestureDetector(
        behavior: HitTestBehavior.translucent,
        onVerticalDragStart: (_) => onVerticalDragStart(),
        onVerticalDragUpdate: (details) =>
            onVerticalDragUpdate(details.delta.dy),
        onVerticalDragEnd: (details) =>
            onVerticalDragEnd(details.velocity.pixelsPerSecond.dy),
        onVerticalDragCancel: onVerticalDragCancel,
        child: Stack(
          clipBehavior: Clip.none,
          fit: StackFit.expand,
          children: [
            Transform.translate(
              key: const Key('wide-navigation-community-current-container'),
              // Move the complete avatar card out of its resting position so
              // the gesture reads as a community change, rather than an image
              // shrinking within a fixed slot.
              offset: Offset(0, outgoingProgress * 32),
              child: Transform.scale(
                scale: 1 - (outgoingProgress * 0.3),
                child: Opacity(
                  key: const Key('wide-navigation-community-current-opacity'),
                  opacity: 1 - outgoingProgress,
                  child: iconImage(
                    iconUrl,
                    const Key('wide-navigation-community-current-icon'),
                  ),
                ),
              ),
            ),
            if (hasNextCommunity)
              Transform.scale(
                key: const Key('wide-navigation-community-incoming-container'),
                scale: 0.1 + (incomingProgress * 0.9),
                child: Opacity(
                  key: const Key('wide-navigation-community-incoming-opacity'),
                  opacity: incomingProgress,
                  child: iconImage(
                    nextIconUrl,
                    const Key('wide-navigation-community-incoming-icon'),
                  ),
                ),
              ),
          ],
        ),
      ),
    );
  }
}

class _WideNavigationProfile extends ConsumerWidget {
  final VoidCallback onTap;

  const _WideNavigationProfile({required this.onTap});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final palette = _wideSidebarPalette(context);
    final profile = ref.watch(profileProvider).value;
    final status = ref.watch(userStatusProvider).asData?.value;
    final displayName = profile?.displayName?.trim();
    final name = displayName != null && displayName.isNotEmpty
        ? displayName
        : 'You';
    final statusEmoji = status?.emoji ?? '';
    final statusText = status?.text.trim() ?? '';
    final statusLabel = [
      if (statusEmoji.isNotEmpty) statusEmoji,
      if (statusText.isNotEmpty) statusText,
    ].join(' ');

    return SizedBox(
      key: const Key('wide-navigation-profile'),
      height: 56,
      child: Row(
        children: [
          Expanded(
            child: Semantics(
              button: true,
              label: 'Open settings for $name',
              child: Material(
                color: Colors.transparent,
                borderRadius: BorderRadius.circular(Radii.md),
                clipBehavior: Clip.antiAlias,
                child: InkWell(
                  key: const Key('wide-navigation-profile-button'),
                  onTap: onTap,
                  child: Padding(
                    padding: const EdgeInsets.only(
                      left: _wideNavigationIdentityAvatarInset,
                      right: Grid.half,
                    ),
                    child: Row(
                      children: [
                        const ProfileAvatar(),
                        const SizedBox(width: _wideNavigationIdentityLabelGap),
                        Expanded(
                          child: Column(
                            mainAxisAlignment: MainAxisAlignment.center,
                            crossAxisAlignment: CrossAxisAlignment.start,
                            children: [
                              Text(
                                name,
                                maxLines: 1,
                                overflow: TextOverflow.ellipsis,
                                style: context.textTheme.bodyMedium?.copyWith(
                                  color: palette.foreground,
                                  fontWeight: FontWeight.w600,
                                ),
                              ),
                              if (statusLabel.isNotEmpty) ...[
                                const SizedBox(height: Grid.quarter),
                                Text(
                                  statusLabel,
                                  maxLines: 1,
                                  overflow: TextOverflow.ellipsis,
                                  style: context.textTheme.labelSmall?.copyWith(
                                    color: palette.mutedForeground,
                                  ),
                                ),
                              ],
                            ],
                          ),
                        ),
                      ],
                    ),
                  ),
                ),
              ),
            ),
          ),
          Tooltip(
            message: 'Settings',
            child: IconButton(
              key: const Key('wide-navigation-settings-button'),
              onPressed: onTap,
              icon: Icon(
                LucideIcons.settings,
                color: palette.mutedForeground,
                size: 20,
              ),
            ),
          ),
        ],
      ),
    );
  }
}

class _WideChannelDestination extends StatelessWidget {
  final Channel channel;
  final String label;
  final dynamic directMessageProfile;
  final bool selected;
  final VoidCallback onTap;

  const _WideChannelDestination({
    required this.channel,
    required this.label,
    this.directMessageProfile,
    required this.selected,
    required this.onTap,
  });

  @override
  Widget build(BuildContext context) {
    final palette = _wideSidebarPalette(context);
    final foregroundColor = selected
        ? palette.activeForeground
        : palette.foreground;
    final icon = channel.isForum ? LucideIcons.fileText : LucideIcons.hash;

    return Semantics(
      key: ValueKey('wide-channel-${channel.id}'),
      button: true,
      selected: selected,
      label: label,
      child: Padding(
        padding: const EdgeInsets.only(bottom: Grid.quarter),
        child: SizedBox(
          height: _wideNavigationChannelRowHeight,
          child: Material(
            color: selected ? palette.activeSurface : Colors.transparent,
            borderRadius: BorderRadius.circular(Radii.sm),
            clipBehavior: Clip.antiAlias,
            child: InkWell(
              onTap: onTap,
              child: Padding(
                padding: const EdgeInsets.symmetric(horizontal: Grid.sm),
                child: Row(
                  children: [
                    SizedBox(
                      width: channel.isDm ? _wideNavigationDmAvatarSize : 22,
                      child: Align(
                        alignment: Alignment.centerLeft,
                        child: channel.isDm
                            ? AvatarImage(
                                imageUrl: directMessageProfile?.avatarUrl,
                                radius: _wideNavigationDmAvatarSize / 2,
                                backgroundColor:
                                    context.colors.primaryContainer,
                                fallback: Text(
                                  directMessageProfile?.initial ?? '?',
                                  textScaler: TextScaler.noScaling,
                                  style: context.textTheme.labelSmall?.copyWith(
                                    color: context.colors.onPrimaryContainer,
                                  ),
                                ),
                              )
                            : Icon(
                                icon,
                                color: foregroundColor,
                                size: _wideNavigationIconSize,
                              ),
                      ),
                    ),
                    SizedBox(
                      width: channel.isDm
                          ? _wideNavigationDmLabelGap
                          : _wideNavigationLabelGap,
                    ),
                    Expanded(
                      child: Text(
                        label,
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                        style: context.textTheme.bodyMedium?.copyWith(
                          color: foregroundColor,
                          fontWeight: selected
                              ? FontWeight.w600
                              : FontWeight.w500,
                        ),
                      ),
                    ),
                  ],
                ),
              ),
            ),
          ),
        ),
      ),
    );
  }
}

class _WideNavigationDestination extends StatelessWidget {
  final WideNavigationDestination destination;
  final bool selected;
  final VoidCallback onTap;

  const _WideNavigationDestination({
    required this.destination,
    required this.selected,
    required this.onTap,
  });

  @override
  Widget build(BuildContext context) {
    final palette = _wideSidebarPalette(context);
    final foregroundColor = selected
        ? palette.activeForeground
        : palette.foreground;
    final label = destination.label == 'Activity' ? 'Inbox' : destination.label;

    return Semantics(
      key: ValueKey('wide-navigation-${label.toLowerCase()}'),
      button: true,
      selected: selected,
      label: label,
      child: Padding(
        // Keep the primary navigation's row rhythm consistent with the
        // channel list directly below it.
        padding: const EdgeInsets.only(bottom: Grid.quarter),
        child: SizedBox(
          height: _wideNavigationPrimaryRowHeight,
          child: Material(
            color: selected ? palette.activeSurface : Colors.transparent,
            borderRadius: BorderRadius.circular(Radii.md),
            clipBehavior: Clip.antiAlias,
            child: InkWell(
              onTap: onTap,
              child: Padding(
                padding: const EdgeInsets.symmetric(horizontal: Grid.sm),
                child: Row(
                  children: [
                    SizedBox(
                      width: 22,
                      child: Align(
                        alignment: Alignment.centerLeft,
                        child: Icon(
                          // The persistent tablet sidebar keeps a stable icon
                          // weight. Selection is expressed by the row's color,
                          // background, and label weight instead.
                          destination.selectedIcon,
                          color: foregroundColor,
                          size: _wideNavigationIconSize,
                        ),
                      ),
                    ),
                    const SizedBox(width: _wideNavigationLabelGap),
                    Expanded(
                      child: Text(
                        label,
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                        style: context.textTheme.bodyMedium?.copyWith(
                          color: foregroundColor,
                          fontWeight: selected
                              ? FontWeight.w600
                              : FontWeight.w500,
                        ),
                      ),
                    ),
                  ],
                ),
              ),
            ),
          ),
        ),
      ),
    );
  }
}

String _wideSidebarChannelLabel(
  Channel channel, {
  required String? currentPubkey,
  required Map<String, dynamic> profiles,
}) {
  if (!channel.isDm) return channel.displayLabel();

  final normalizedCurrent = currentPubkey?.toLowerCase();
  final labels = <String>[];
  for (var index = 0; index < channel.participantPubkeys.length; index++) {
    final pubkey = channel.participantPubkeys[index].toLowerCase();
    if (pubkey == normalizedCurrent) continue;
    final profile = profiles[pubkey];
    final displayName = profile?.displayName?.trim();
    if (displayName != null && displayName.isNotEmpty) {
      labels.add(displayName);
    } else if (index < channel.participants.length &&
        channel.participants[index].trim().isNotEmpty) {
      labels.add(channel.participants[index]);
    }
  }

  if (labels.isNotEmpty) return formatDmParticipantDisplayName(labels);
  return resolveDmChannelDisplayLabel(channel, currentPubkey: currentPubkey);
}

dynamic _wideSidebarDirectMessageProfile(
  Channel channel, {
  required String? currentPubkey,
  required Map<String, dynamic> profiles,
}) {
  if (!channel.isDm) return null;

  final normalizedCurrent = currentPubkey?.toLowerCase();
  for (final pubkey in channel.participantPubkeys) {
    final normalizedPubkey = pubkey.toLowerCase();
    if (normalizedPubkey == normalizedCurrent) continue;
    final profile = profiles[normalizedPubkey];
    if (profile != null) return profile;
  }
  return null;
}
