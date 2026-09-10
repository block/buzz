import 'dart:async';
import 'dart:ui';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';

import '../../shared/theme/theme.dart';
import '../../shared/widgets/directional_transition_scope.dart';
import '../../shared/widgets/mobile_tab_footer_backdrop.dart';
import '../activity/activity_page.dart';
import '../channels/channel.dart';
import '../channels/channel_detail_page.dart';
import '../channels/channels_page.dart';
import '../profile/profile_avatar.dart';
import '../search/search_page.dart';

/// Minimum available width at which Buzz moves primary navigation to a rail.
const double homeNavigationRailBreakpoint = 900;
const double _kExpandedRailControlSize = 56;
const double _kExpandedRailAvatarSize = 48;

class HomePage extends HookConsumerWidget {
  const HomePage({
    required this.settingsPageBuilder,
    required this.hasUnreadInbox,
    super.key,
  });

  final WidgetBuilder settingsPageBuilder;
  final bool hasUnreadInbox;

  static const double _tabBarHeight = mobileTabBarHeight;
  static const double _tabBarRadius = _tabBarHeight / 2;
  static const double _tabBarInnerInset = Grid.half;
  static const double _selectedTabRadius =
      (_tabBarHeight - (_tabBarInnerInset * 2)) / 2;
  static const double _tabBarBottomGap = mobileTabBarBottomGap;
  static const double _tabBarHorizontalMargin = Grid.gutter;
  static const double _tabDestinationHorizontalPadding = Grid.sm;
  static const double _tabIconSize = 22;
  static const double _fabClearance = _tabBarHeight + _tabBarBottomGap;
  static const Duration _tabIconWeightDuration = Duration(milliseconds: 120);
  static const Duration _tabUnreadBadgeDuration = Duration(milliseconds: 220);
  static const double _settingsBackgroundScale = 0.97;
  static const Duration _tabContentTransitionDuration = Duration(
    milliseconds: 240,
  );
  static const Curve _tabContentTransitionCurve = Cubic(0.22, 1, 0.36, 1);
  static const double _tabContentTransitionDistance = 24;

  static const _destinations = [
    _HomeDestination(
      icon: LucideIcons.house300,
      selectedIcon: LucideIcons.house500,
      label: 'Home',
    ),
    _HomeDestination(
      icon: LucideIcons.inbox300,
      selectedIcon: LucideIcons.inbox500,
      label: 'Activity',
    ),
    _HomeDestination(
      icon: LucideIcons.search300,
      selectedIcon: LucideIcons.search500,
      label: 'Search',
    ),
  ];

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final tabIndex = useState(0);
    final visitedTabs = useRef(<int>{0});
    final tabContentTransitionDirection = useRef(1.0);
    final tabContentTransitionController = useAnimationController(
      duration: _tabContentTransitionDuration,
      initialValue: 1,
    );
    final tabContentTransitionValue = useAnimation(
      tabContentTransitionController,
    );
    final homeReselection = useValueNotifier(0);
    final activityReselection = useValueNotifier(0);
    final searchReselection = useValueNotifier(0);
    final selectedWideChannel = useState<Channel?>(null);
    final settingsTransitionProgress = useValueNotifier(0.0);
    final reducedMotion = MediaQuery.of(context).disableAnimations;
    final usesNavigationRail =
        MediaQuery.sizeOf(context).width >= homeNavigationRailBreakpoint;
    final tabContentTransitionProgress = reducedMotion
        ? 1.0
        : _tabContentTransitionCurve.transform(tabContentTransitionValue);
    final systemBottomInset = MediaQuery.paddingOf(context).bottom;
    final navigationBarWidth = _floatingTabBarWidth(
      MediaQuery.sizeOf(context).width,
      _destinations.length,
    );

    Future<void> openChannel(Channel channel) async {
      if (usesNavigationRail) {
        selectedWideChannel.value = channel;
        return;
      }
      await Navigator.of(context).push(
        MaterialPageRoute<void>(
          builder: (_) => ChannelDetailPage(channel: channel),
        ),
      );
    }

    void reportSettingsTransitionProgress(double progress) {
      if (settingsTransitionProgress.value != progress) {
        settingsTransitionProgress.value = progress;
      }
    }

    void openSettings() => openSettingsPage(
      context: context,
      builder: settingsPageBuilder,
      onTransitionProgress: reportSettingsTransitionProgress,
    );

    void openCommunitySwitcher() =>
        showCommunitySwitcher(context: context, ref: ref);

    void selectDestination(int index) {
      if (index == tabIndex.value) {
        if (index == 0 && selectedWideChannel.value != null) {
          selectedWideChannel.value = null;
          return;
        }
        switch (index) {
          case 0:
            homeReselection.value++;
          case 1:
            activityReselection.value++;
          case 2:
            searchReselection.value++;
        }
        return;
      }
      selectedWideChannel.value = null;
      tabContentTransitionDirection.value = index > tabIndex.value ? 1 : -1;
      unawaited(HapticFeedback.selectionClick());
      visitedTabs.value.add(index);
      tabIndex.value = index;
      if (reducedMotion) {
        tabContentTransitionController.value = 1;
      } else {
        unawaited(tabContentTransitionController.forward(from: 0));
      }
    }

    final pages = [
      ChannelsPage(
        settingsPageBuilder: settingsPageBuilder,
        onOpenChannel: openChannel,
        showCommunityAction: !usesNavigationRail,
        showProfileAction: !usesNavigationRail,
        showTopBar: !usesNavigationRail,
        tabReselection: homeReselection,
        onSettingsTransitionProgress: reportSettingsTransitionProgress,
      ),
      if (visitedTabs.value.contains(1))
        ActivityPage(tabReselection: activityReselection)
      else
        const SizedBox.shrink(),
      if (visitedTabs.value.contains(2))
        SearchPage(tabReselection: searchReselection)
      else
        const SizedBox.shrink(),
    ];

    final settingsTransitionGradient = tabIndex.value == 0
        ? context.appColors.topSectionGradient
        : null;
    final visibleWideChannel = usesNavigationRail && tabIndex.value == 0
        ? selectedWideChannel.value
        : null;
    final homeContent = _HomeContent(
      tabIndex: tabIndex.value,
      pages: pages,
      tabContentTransitionDirection: tabContentTransitionDirection.value,
      tabContentTransitionProgress: tabContentTransitionProgress,
      usesNavigationRail: usesNavigationRail,
      navigationBarWidth: navigationBarWidth,
      systemBottomInset: systemBottomInset,
      onOpenChannel: openChannel,
    );
    final homeBody = Row(
      children: [
        if (usesNavigationRail)
          _ExpandedHomeNavigation(
            selectedIndex: tabIndex.value,
            hasUnreadInbox: hasUnreadInbox,
            onDestinationSelected: selectDestination,
            onOpenCommunity: openCommunitySwitcher,
            onOpenProfile: openSettings,
            destinations: _destinations,
          ),
        Expanded(
          child: visibleWideChannel == null
              ? homeContent
              : Row(
                  key: const ValueKey('wide-channel-split-view'),
                  children: [
                    Expanded(flex: 1, child: homeContent),
                    VerticalDivider(
                      width: 1,
                      thickness: 1,
                      color: context.colors.outlineVariant,
                    ),
                    Expanded(
                      flex: 2,
                      child: ChannelDetailPage(
                        key: ValueKey(
                          'wide-channel-detail-${visibleWideChannel.id}',
                        ),
                        channel: visibleWideChannel,
                      ),
                    ),
                  ],
                ),
        ),
      ],
    );

    return Stack(
      fit: StackFit.expand,
      children: [
        Positioned.fill(
          child: DecoratedBox(
            key: const ValueKey('home-settings-transition-backdrop'),
            decoration: BoxDecoration(
              color: settingsTransitionGradient == null
                  ? context.colors.surface
                  : null,
              gradient: settingsTransitionGradient,
            ),
          ),
        ),
        ValueListenableBuilder<double>(
          key: const ValueKey('home-settings-transition-progress'),
          // Keep one stable listenable for Home. Swapping in the route's
          // animation can briefly rebuild with its completed value before the
          // new controller starts, which makes the background scale twice.
          valueListenable: settingsTransitionProgress,
          child: Scaffold(
            backgroundColor: Colors.transparent,
            // Keep the floating navigation and Home quick actions anchored while the
            // keyboard is visible on any tab.
            resizeToAvoidBottomInset: false,
            extendBody: true,
            body: homeBody,
            bottomNavigationBar: usesNavigationRail
                ? null
                : _FloatingTabBar(
                    selectedIndex: tabIndex.value,
                    hasUnreadInbox: hasUnreadInbox,
                    onDestinationSelected: selectDestination,
                    destinations: _destinations,
                  ),
          ),
          builder: (context, progress, child) {
            final curvedProgress = reducedMotion
                ? 0.0
                : Curves.easeOutCubic.transform(progress);
            return Opacity(
              key: const ValueKey('home-settings-transition-opacity'),
              // Settings supplies the crossfade. Keeping Home opaque beneath
              // it prevents the bare backdrop showing through two partially
              // transparent layers.
              opacity: 1,
              child: Transform.scale(
                key: const ValueKey('home-settings-transition-scale'),
                scale: lerpDouble(1, _settingsBackgroundScale, curvedProgress),
                alignment: Alignment.center,
                child: child,
              ),
            );
          },
        ),
      ],
    );
  }
}

class _HomeContent extends StatelessWidget {
  final int tabIndex;
  final List<Widget> pages;
  final double tabContentTransitionDirection;
  final double tabContentTransitionProgress;
  final bool usesNavigationRail;
  final double navigationBarWidth;
  final double systemBottomInset;
  final Future<void> Function(Channel channel) onOpenChannel;

  const _HomeContent({
    required this.tabIndex,
    required this.pages,
    required this.tabContentTransitionDirection,
    required this.tabContentTransitionProgress,
    required this.usesNavigationRail,
    required this.navigationBarWidth,
    required this.systemBottomInset,
    required this.onOpenChannel,
  });

  @override
  Widget build(BuildContext context) {
    return LayoutBuilder(
      builder: (context, constraints) {
        final contentSize = constraints.biggest;
        return SizedBox.expand(
          child: Stack(
            fit: StackFit.expand,
            children: [
              Positioned.fill(child: ColoredBox(color: context.colors.surface)),
              Positioned.fill(
                child: MediaQuery(
                  data: _mediaQueryForHomeContent(
                    context,
                    contentSize: contentSize,
                    bottomClearance: usesNavigationRail
                        ? 0
                        : HomePage._fabClearance,
                    removeLeadingInset: usesNavigationRail,
                  ),
                  child: DirectionalTransitionScope(
                    horizontalOffset:
                        tabContentTransitionDirection *
                        HomePage._tabContentTransitionDistance *
                        (1 - tabContentTransitionProgress),
                    opacity: tabContentTransitionProgress,
                    child: ClipRect(
                      child: IndexedStack(
                        key: const ValueKey('home-destination-pages'),
                        index: tabIndex,
                        children: pages,
                      ),
                    ),
                  ),
                ),
              ),
              if (!usesNavigationRail)
                Align(
                  alignment: Alignment.bottomCenter,
                  child: IgnorePointer(
                    child: MobileTabFooterBackdrop(
                      height: mobileTabFooterBackdropHeight(context),
                      tint: context.colors.primaryContainer,
                    ),
                  ),
                ),
              Positioned.fill(
                child: MediaQuery(
                  data: MediaQuery.of(context).copyWith(size: contentSize),
                  child: ChannelQuickActionsLauncher(
                    visible: tabIndex == 0,
                    navigationBarHeight: HomePage._tabBarHeight,
                    navigationBarBottomGap: HomePage._tabBarBottomGap,
                    navigationBarWidth: usesNavigationRail
                        ? 0
                        : navigationBarWidth,
                    systemBottomInset: systemBottomInset,
                    rightInset: Grid.sm,
                    maxOpenWidth: usesNavigationRail ? 430 : double.infinity,
                    onOpenChannel: onOpenChannel,
                  ),
                ),
              ),
            ],
          ),
        );
      },
    );
  }
}

class _ExpandedHomeNavigation extends StatelessWidget {
  final int selectedIndex;
  final bool hasUnreadInbox;
  final ValueChanged<int> onDestinationSelected;
  final VoidCallback onOpenCommunity;
  final VoidCallback onOpenProfile;
  final List<_HomeDestination> destinations;

  const _ExpandedHomeNavigation({
    required this.selectedIndex,
    required this.hasUnreadInbox,
    required this.onDestinationSelected,
    required this.onOpenCommunity,
    required this.onOpenProfile,
    required this.destinations,
  });

  @override
  Widget build(BuildContext context) {
    return DecoratedBox(
      key: const ValueKey('expanded-home-navigation'),
      decoration: BoxDecoration(
        color: context.colors.surface,
        border: Border(right: BorderSide(color: context.colors.outlineVariant)),
      ),
      child: SafeArea(
        right: false,
        child: Column(
          children: [
            Padding(
              padding: const EdgeInsets.symmetric(vertical: Grid.xxs),
              child: SizedBox.square(
                key: const ValueKey('expanded-home-community'),
                dimension: _kExpandedRailControlSize,
                child: Center(
                  child: CommunityNavigationAvatar(
                    size: _kExpandedRailAvatarSize,
                    onTap: onOpenCommunity,
                  ),
                ),
              ),
            ),
            Expanded(
              child: NavigationRail(
                backgroundColor: Colors.transparent,
                selectedIndex: selectedIndex,
                onDestinationSelected: onDestinationSelected,
                labelType: NavigationRailLabelType.all,
                groupAlignment: -1,
                useIndicator: true,
                indicatorColor: context.colors.primaryContainer,
                selectedIconTheme: IconThemeData(color: context.colors.primary),
                unselectedIconTheme: IconThemeData(
                  color: context.colors.onSurfaceVariant,
                ),
                selectedLabelTextStyle: context.textTheme.labelMedium?.copyWith(
                  color: context.colors.primary,
                  fontWeight: FontWeight.w600,
                ),
                unselectedLabelTextStyle: context.textTheme.labelMedium
                    ?.copyWith(color: context.colors.onSurfaceVariant),
                destinations: [
                  for (var index = 0; index < destinations.length; index++)
                    NavigationRailDestination(
                      icon: _NavigationRailIcon(
                        icon: destinations[index].icon,
                        showUnreadBadge: index == 1 && hasUnreadInbox,
                      ),
                      selectedIcon: _NavigationRailIcon(
                        icon: destinations[index].selectedIcon,
                        showUnreadBadge: index == 1 && hasUnreadInbox,
                      ),
                      label: Text(
                        destinations[index].label,
                        semanticsLabel: index == 1 && hasUnreadInbox
                            ? '${destinations[index].label}, unread'
                            : destinations[index].label,
                      ),
                    ),
                ],
              ),
            ),
            Padding(
              padding: EdgeInsets.only(
                bottom: _expandedRailProfileBottomPadding(context),
              ),
              child: Semantics(
                button: true,
                label: 'Profile and settings',
                child: Tooltip(
                  message: 'Profile and settings',
                  child: SizedBox.square(
                    key: const ValueKey('expanded-home-profile'),
                    dimension: _kExpandedRailControlSize,
                    child: Center(
                      child: ProfileAvatar(
                        size: _kExpandedRailAvatarSize,
                        showPresence: false,
                        onTap: onOpenProfile,
                      ),
                    ),
                  ),
                ),
              ),
            ),
          ],
        ),
      ),
    );
  }
}

double _expandedRailProfileBottomPadding(BuildContext context) {
  final systemBottomInset = MediaQuery.paddingOf(context).bottom;
  final navigationBottomInset = systemBottomInset > HomePage._tabBarBottomGap
      ? systemBottomInset
      : HomePage._tabBarBottomGap;
  final createButtonBottomInset =
      navigationBottomInset +
      ((HomePage._tabBarHeight - _kExpandedRailControlSize) / 2);
  return createButtonBottomInset - systemBottomInset;
}

class _NavigationRailIcon extends StatelessWidget {
  final IconData icon;
  final bool showUnreadBadge;

  const _NavigationRailIcon({
    required this.icon,
    required this.showUnreadBadge,
  });

  @override
  Widget build(BuildContext context) {
    return Stack(
      clipBehavior: Clip.none,
      children: [
        Icon(icon, size: HomePage._tabIconSize),
        if (showUnreadBadge)
          Positioned(
            top: -4,
            right: -5,
            child: Container(
              key: const ValueKey('activity-rail-unread-dot'),
              width: 10,
              height: 10,
              decoration: BoxDecoration(
                color: context.colors.error,
                shape: BoxShape.circle,
                border: Border.all(color: context.colors.surface, width: 1.5),
              ),
            ),
          ),
      ],
    );
  }
}

double _floatingTabDestinationWidth(double screenWidth, int destinationCount) {
  final preferredDestinationWidth =
      HomePage._tabIconSize + (HomePage._tabDestinationHorizontalPadding * 2);
  final availableInnerWidth =
      screenWidth -
      (HomePage._tabBarHorizontalMargin * 2) -
      (HomePage._tabBarInnerInset * 2);
  return preferredDestinationWidth
      .clamp(0.0, availableInnerWidth / destinationCount)
      .toDouble();
}

double _floatingTabBarWidth(double screenWidth, int destinationCount) {
  if (destinationCount <= 0) return 0;
  return (_floatingTabDestinationWidth(screenWidth, destinationCount) *
          destinationCount) +
      (HomePage._tabBarInnerInset * 2);
}

MediaQueryData _mediaQueryWithFloatingTabBarClearance(
  BuildContext context,
  double clearance,
) {
  final mediaQuery = MediaQuery.of(context);
  return mediaQuery.copyWith(
    padding: mediaQuery.padding.copyWith(
      bottom: mediaQuery.padding.bottom + clearance,
    ),
    viewPadding: mediaQuery.viewPadding.copyWith(
      bottom: mediaQuery.viewPadding.bottom + clearance,
    ),
  );
}

MediaQueryData _mediaQueryForHomeContent(
  BuildContext context, {
  required Size contentSize,
  required double bottomClearance,
  required bool removeLeadingInset,
}) {
  final mediaQuery = _mediaQueryWithFloatingTabBarClearance(
    context,
    bottomClearance,
  );
  if (!removeLeadingInset) {
    return mediaQuery.copyWith(size: contentSize);
  }
  return mediaQuery.copyWith(
    size: contentSize,
    padding: mediaQuery.padding.copyWith(left: 0),
    viewPadding: mediaQuery.viewPadding.copyWith(left: 0),
  );
}

class _HomeDestination {
  final IconData icon;
  final IconData selectedIcon;
  final String label;

  const _HomeDestination({
    required this.icon,
    required this.selectedIcon,
    required this.label,
  });
}

class _FloatingTabBar extends StatelessWidget {
  final int selectedIndex;
  final bool hasUnreadInbox;
  final ValueChanged<int> onDestinationSelected;
  final List<_HomeDestination> destinations;

  const _FloatingTabBar({
    required this.selectedIndex,
    required this.hasUnreadInbox,
    required this.onDestinationSelected,
    required this.destinations,
  });

  @override
  Widget build(BuildContext context) {
    final colorScheme = context.colors;
    final isDark = context.theme.brightness == Brightness.dark;
    final reducedMotion = MediaQuery.of(context).disableAnimations;
    if (destinations.isEmpty) {
      return const SizedBox.shrink();
    }
    final destinationCount = destinations.length;
    final safeSelectedIndex = selectedIndex
        .clamp(0, destinationCount - 1)
        .toInt();
    final selectedAlignment = destinationCount <= 1
        ? Alignment.center
        : Alignment(-1 + (2 * safeSelectedIndex / (destinationCount - 1)), 0);

    final destinationWidth = _floatingTabDestinationWidth(
      MediaQuery.sizeOf(context).width,
      destinationCount,
    );

    return SafeArea(
      key: const ValueKey('compact-home-navigation'),
      minimum: const EdgeInsets.fromLTRB(
        HomePage._tabBarHorizontalMargin,
        0,
        HomePage._tabBarHorizontalMargin,
        HomePage._tabBarBottomGap,
      ),
      child: Align(
        alignment: Alignment.bottomCenter,
        heightFactor: 1,
        child: DecoratedBox(
          decoration: BoxDecoration(
            borderRadius: BorderRadius.circular(HomePage._tabBarRadius),
            boxShadow: [
              BoxShadow(
                color: colorScheme.shadow.withValues(alpha: 0.10),
                blurRadius: 20,
                offset: const Offset(0, 8),
              ),
            ],
          ),
          child: ClipRRect(
            borderRadius: BorderRadius.circular(HomePage._tabBarRadius),
            child: BackdropFilter(
              filter: ImageFilter.blur(sigmaX: 18, sigmaY: 18),
              child: DecoratedBox(
                decoration: BoxDecoration(
                  borderRadius: BorderRadius.circular(HomePage._tabBarRadius),
                  color: isDark
                      ? colorScheme.surfaceContainerHighest.withValues(
                          alpha: 0.72,
                        )
                      : colorScheme.surface,
                  border: Border.all(
                    color: colorScheme.outlineVariant.withValues(
                      alpha: isDark ? 0.20 : 0.38,
                    ),
                  ),
                ),
                child: Padding(
                  padding: const EdgeInsets.all(HomePage._tabBarInnerInset),
                  child: SizedBox(
                    height:
                        HomePage._tabBarHeight -
                        (HomePage._tabBarInnerInset * 2),
                    width: destinationWidth * destinationCount,
                    child: Stack(
                      children: [
                        AnimatedAlign(
                          alignment: selectedAlignment,
                          duration: reducedMotion
                              ? Duration.zero
                              : const Duration(milliseconds: 180),
                          curve: Curves.easeOutCubic,
                          child: SizedBox(
                            width: destinationWidth,
                            height: double.infinity,
                            child: DecoratedBox(
                              decoration: BoxDecoration(
                                color: colorScheme.primaryContainer,
                                borderRadius: BorderRadius.circular(
                                  HomePage._selectedTabRadius,
                                ),
                              ),
                            ),
                          ),
                        ),
                        Row(
                          crossAxisAlignment: CrossAxisAlignment.stretch,
                          children: [
                            for (var i = 0; i < destinations.length; i++)
                              SizedBox(
                                width: destinationWidth,
                                child: _FloatingTabDestination(
                                  destination: destinations[i],
                                  selected: i == safeSelectedIndex,
                                  showUnreadBadge: i == 1 && hasUnreadInbox,
                                  onTap: () => onDestinationSelected(i),
                                ),
                              ),
                          ],
                        ),
                      ],
                    ),
                  ),
                ),
              ),
            ),
          ),
        ),
      ),
    );
  }
}

class _FloatingTabDestination extends StatelessWidget {
  final _HomeDestination destination;
  final bool selected;
  final bool showUnreadBadge;
  final VoidCallback onTap;

  const _FloatingTabDestination({
    required this.destination,
    required this.selected,
    required this.showUnreadBadge,
    required this.onTap,
  });

  @override
  Widget build(BuildContext context) {
    final colorScheme = context.colors;
    final isDark = context.theme.brightness == Brightness.dark;
    final reducedMotion = MediaQuery.of(context).disableAnimations;
    final foregroundColor = selected
        ? colorScheme.onPrimaryContainer
        : colorScheme.onSurfaceVariant;
    final icon = selected ? destination.selectedIcon : destination.icon;
    final badgeOutlineColor = selected
        ? colorScheme.primaryContainer
        : (isDark ? colorScheme.surfaceContainerHighest : colorScheme.surface);

    final showVisibleUnreadBadge = showUnreadBadge && !selected;

    return Semantics(
      button: true,
      selected: selected,
      label: showVisibleUnreadBadge
          ? '${destination.label}, unread'
          : destination.label,
      child: Tooltip(
        message: destination.label,
        excludeFromSemantics: true,
        child: Material(
          color: Colors.transparent,
          clipBehavior: Clip.antiAlias,
          borderRadius: BorderRadius.circular(HomePage._selectedTabRadius),
          child: InkWell(
            onTap: onTap,
            overlayColor: const WidgetStatePropertyAll<Color>(
              Colors.transparent,
            ),
            borderRadius: BorderRadius.circular(HomePage._selectedTabRadius),
            child: Center(
              child: SizedBox(
                width: HomePage._tabIconSize + 8,
                height: HomePage._tabIconSize + 8,
                child: Stack(
                  clipBehavior: Clip.none,
                  children: [
                    Center(
                      child: AnimatedSwitcher(
                        duration: reducedMotion
                            ? Duration.zero
                            : HomePage._tabIconWeightDuration,
                        switchInCurve: Curves.easeOutCubic,
                        switchOutCurve: Curves.easeOutCubic,
                        transitionBuilder: (child, animation) =>
                            FadeTransition(opacity: animation, child: child),
                        child: Icon(
                          icon,
                          key: ValueKey('${destination.label}-$icon'),
                          color: foregroundColor,
                          size: HomePage._tabIconSize,
                        ),
                      ),
                    ),
                    if (showUnreadBadge)
                      Positioned(
                        top: 0,
                        right: 0,
                        child: AnimatedScale(
                          key: const ValueKey('activity-tab-unread-dot-scale'),
                          scale: selected ? 0 : 1,
                          alignment: const Alignment(-0.5, 0.5),
                          duration: reducedMotion
                              ? Duration.zero
                              : HomePage._tabUnreadBadgeDuration,
                          curve: Curves.easeOutCubic,
                          child: Container(
                            key: const ValueKey('activity-tab-unread-dot'),
                            width: 12,
                            height: 12,
                            padding: const EdgeInsets.all(2),
                            decoration: BoxDecoration(
                              color: badgeOutlineColor,
                              shape: BoxShape.circle,
                            ),
                            child: DecoratedBox(
                              decoration: BoxDecoration(
                                color: colorScheme.primary,
                                shape: BoxShape.circle,
                              ),
                            ),
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
