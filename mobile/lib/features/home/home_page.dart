import 'dart:async';
import 'dart:ui';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';

import '../../shared/community/community_provider.dart';
import '../../shared/theme/theme.dart';
import '../../shared/widgets/mobile_tab_footer_backdrop.dart';
import '../../shared/widgets/skeleton.dart';
import '../activity/activity_page.dart';
import '../activity/activity_provider.dart';
import '../channels/channels_page.dart';
import '../search/search_page.dart';

part 'home_page/wide_navigation_skeletons.dart';

class HomePage extends HookConsumerWidget {
  const HomePage({required this.settingsPageBuilder, super.key});

  final WidgetBuilder settingsPageBuilder;

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
  static const double _wideNavigationBreakpoint = 840;
  static const double _wideContentInset = Grid.half + Grid.quarter;

  static const _destinations = [
    WideNavigationDestination(
      icon: LucideIcons.house300,
      selectedIcon: LucideIcons.house500,
      label: 'Home',
    ),
    WideNavigationDestination(
      icon: LucideIcons.inbox300,
      selectedIcon: LucideIcons.inbox500,
      label: 'Activity',
    ),
    WideNavigationDestination(
      icon: LucideIcons.search300,
      selectedIcon: LucideIcons.search500,
      label: 'Search',
    ),
  ];

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final isWide =
        MediaQuery.sizeOf(context).width >= HomePage._wideNavigationBreakpoint;
    // On iPad, the persistent sidebar replaces the phone's Home tab. Start at
    // Inbox, which is the first top-level destination in that layout.
    final tabIndex = useState(isWide ? 1 : 0);
    final selectedChannelId = useState<String?>(null);
    final pendingCommunityId = useState<String?>(null);
    // Clear the tablet selection as soon as the active-community state begins
    // changing. The channel provider intentionally retains its last result
    // while the next relay connects, so retaining the selection here could
    // briefly render (and query) an old-community channel in the new workspace.
    final activeCommunityId = ref.watch(
      activeCommunityProvider.select(
        (value) => value.unwrapPrevious().value?.id,
      ),
    );
    final selectedChannelCommunityId = useRef<String?>(activeCommunityId);
    if (selectedChannelCommunityId.value != activeCommunityId) {
      selectedChannelId.value = null;
      selectedChannelCommunityId.value = activeCommunityId;
    }
    final isCommunitySwitching = pendingCommunityId.value != null;
    final activityAsync = ref.watch(activityProvider);
    final isActivitySettled =
        !activityAsync.isLoading &&
        (activityAsync.hasValue || activityAsync.hasError);
    final systemBottomInset = MediaQuery.paddingOf(context).bottom;
    final navigationBarWidth = _floatingTabBarWidth(
      MediaQuery.sizeOf(context).width,
      _destinations.length,
    );
    final useSidebarLayout = isWide;

    final pages = [
      if (isWide && selectedChannelId.value != null)
        WideChannelContent(
          channelId: selectedChannelId.value!,
          onChannelLeft: () => selectedChannelId.value = null,
        )
      else
        ChannelsPage(settingsPageBuilder: settingsPageBuilder),
      const ActivityPage(),
      const SearchPage(),
    ];
    // The phone's Home destination is represented by index zero. On a tablet,
    // that destination only exists while a channel is selected; otherwise
    // returning from a nested route must land back in Inbox rather than reveal
    // the phone's ChannelsPage inside the desktop-style workspace.
    final wideFallbackToInbox =
        isWide && selectedChannelId.value == null && tabIndex.value == 0;
    final wideContentIndex = wideFallbackToInbox ? 1 : tabIndex.value;
    final wideSidebarSelection = selectedChannelId.value != null
        ? null
        : wideFallbackToInbox
        ? 1
        : tabIndex.value;

    return Scaffold(
      backgroundColor: Colors.transparent,
      // Keep the floating navigation and Home quick actions anchored while the
      // keyboard is visible on any tab.
      resizeToAvoidBottomInset: false,
      extendBody: true,
      body: SizedBox.expand(
        child: Stack(
          fit: StackFit.expand,
          children: [
            Positioned.fill(
              child: useSidebarLayout
                  ? DecoratedBox(
                      decoration: BoxDecoration(
                        color: context.appColors.topSectionGradient == null
                            ? context.colors.surfaceContainerLowest
                            : null,
                        gradient: context.appColors.topSectionGradient,
                      ),
                    )
                  : ColoredBox(color: context.colors.surface),
            ),
            Positioned.fill(
              child: useSidebarLayout
                  ? Row(
                      children: [
                        WideChannelsNavigation(
                          selectedIndex: wideSidebarSelection,
                          onDestinationSelected: (index) {
                            final hadSelectedChannel =
                                selectedChannelId.value != null;
                            if (index == tabIndex.value &&
                                !(hadSelectedChannel && index != 0)) {
                              return;
                            }
                            // Search and Inbox replace a selected channel in
                            // the tablet workspace, so they must also clear
                            // the channel selection that owns the sidebar
                            // highlight.
                            selectedChannelId.value = null;
                            unawaited(HapticFeedback.selectionClick());
                            tabIndex.value = index;
                          },
                          onChannelSelected: (channelId) {
                            selectedChannelId.value = channelId;
                            if (tabIndex.value != 0) {
                              unawaited(HapticFeedback.selectionClick());
                            }
                            tabIndex.value = 0;
                          },
                          selectedChannelId: selectedChannelId.value,
                          onProfileSelected: () => unawaited(
                            Navigator.of(context).push(
                              MaterialPageRoute<void>(
                                builder: settingsPageBuilder,
                              ),
                            ),
                          ),
                          isCommunitySwitching: isCommunitySwitching,
                          onCommunitySwitchStart: (communityId) {
                            pendingCommunityId.value = communityId;
                          },
                          pendingCommunityId: pendingCommunityId.value,
                          isActivitySettled: isActivitySettled,
                          onCommunitySwitchComplete: () {
                            pendingCommunityId.value = null;
                          },
                          destinations: _destinations,
                        ),
                        Expanded(
                          child: Padding(
                            key: const Key('wide-navigation-content-inset'),
                            padding: const EdgeInsets.only(
                              top: HomePage._wideContentInset,
                              right: HomePage._wideContentInset,
                              bottom: HomePage._wideContentInset,
                            ),
                            child: DecoratedBox(
                              key: const Key('wide-navigation-content-surface'),
                              decoration: BoxDecoration(
                                color: context.colors.surface,
                                borderRadius: BorderRadius.circular(
                                  Radii.dialog,
                                ),
                                // Matches desktop's Buzz content surface: a
                                // hairline on the upper-left edge plus a very
                                // soft lift into the exposed gradient.
                                boxShadow:
                                    context.theme.brightness == Brightness.light
                                    ? [
                                        BoxShadow(
                                          color: context.colors.outlineVariant
                                              .withValues(alpha: 0.45),
                                          offset: const Offset(-1, -1),
                                        ),
                                        BoxShadow(
                                          color: Colors.black.withValues(
                                            alpha: 0.07,
                                          ),
                                          blurRadius: 4,
                                        ),
                                      ]
                                    : null,
                              ),
                              child: ClipRRect(
                                borderRadius: BorderRadius.circular(
                                  Radii.dialog,
                                ),
                                child: MediaQuery(
                                  // The desktop-like canvas is inset below the
                                  // system top edge. Remove that same amount
                                  // from the workspace's top safe padding so
                                  // its titles stay aligned with the sidebar
                                  // community switcher.
                                  data: MediaQuery.of(context).copyWith(
                                    padding: MediaQuery.paddingOf(context)
                                        .copyWith(
                                          top:
                                              (MediaQuery.paddingOf(
                                                        context,
                                                      ).top -
                                                      HomePage
                                                          ._wideContentInset)
                                                  .clamp(0, double.infinity),
                                        ),
                                  ),
                                  child: SkeletonReveal(
                                    loading: isCommunitySwitching,
                                    skeleton: const _WideCommunityContentSkeleton(
                                      key: Key(
                                        'wide-community-switch-content-skeleton',
                                      ),
                                    ),
                                    content: IndexedStack(
                                      index: wideContentIndex,
                                      children: pages,
                                    ),
                                  ),
                                ),
                              ),
                            ),
                          ),
                        ),
                      ],
                    )
                  : MediaQuery(
                      data: _mediaQueryWithFloatingTabBarClearance(
                        context,
                        HomePage._fabClearance,
                      ),
                      child: IndexedStack(
                        index: tabIndex.value,
                        children: pages,
                      ),
                    ),
            ),
            if (!useSidebarLayout)
              Align(
                alignment: Alignment.bottomCenter,
                child: IgnorePointer(
                  child: MobileTabFooterBackdrop(
                    height: mobileTabFooterBackdropHeight(context),
                  ),
                ),
              ),
            Positioned.fill(
              child: useSidebarLayout
                  ? const SizedBox.shrink()
                  : ChannelQuickActionsLauncher(
                      visible: tabIndex.value == 0,
                      navigationBarHeight: HomePage._tabBarHeight,
                      navigationBarBottomGap: HomePage._tabBarBottomGap,
                      navigationBarWidth: navigationBarWidth,
                      systemBottomInset: systemBottomInset,
                      rightInset: Grid.sm,
                    ),
            ),
          ],
        ),
      ),
      bottomNavigationBar: useSidebarLayout
          ? null
          : _FloatingTabBar(
              selectedIndex: tabIndex.value,
              onDestinationSelected: (i) {
                if (i == tabIndex.value) return;
                unawaited(HapticFeedback.selectionClick());
                tabIndex.value = i;
              },
              destinations: _destinations,
            ),
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

class _FloatingTabBar extends StatelessWidget {
  final int selectedIndex;
  final ValueChanged<int> onDestinationSelected;
  final List<WideNavigationDestination> destinations;

  const _FloatingTabBar({
    required this.selectedIndex,
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
      key: const Key('floating-tab-bar'),
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
  final WideNavigationDestination destination;
  final bool selected;
  final VoidCallback onTap;

  const _FloatingTabDestination({
    required this.destination,
    required this.selected,
    required this.onTap,
  });

  @override
  Widget build(BuildContext context) {
    final colorScheme = context.colors;
    final reducedMotion = MediaQuery.of(context).disableAnimations;
    final foregroundColor = selected
        ? colorScheme.onPrimaryContainer
        : colorScheme.onSurfaceVariant;
    final icon = selected ? destination.selectedIcon : destination.icon;

    return Semantics(
      button: true,
      selected: selected,
      label: destination.label,
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
          ),
        ),
      ),
    );
  }
}
