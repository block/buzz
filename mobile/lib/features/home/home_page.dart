import 'dart:async';
import 'dart:ui';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';

import '../../shared/theme/theme.dart';
import '../../shared/widgets/mobile_tab_footer_backdrop.dart';
import '../activity/activity_page.dart';
import '../agents/agents_page.dart';
import '../channels/channel.dart';
import '../channels/channel_detail_page.dart';
import '../channels/channels_page.dart';
import '../channels/channels_provider.dart';
import '../profile/profile_avatar.dart';
import '../profile/profile_provider.dart';
import '../search/search_page.dart';

part 'tablet_workspace_sidebar.dart';

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
  static const double _tabletBreakpoint = 600;
  static const double _tabletRailWidth = Grid.xxxl;
  static const double _workspaceSidebarWidth = 280;
  static const double _workspaceDividerWidth = 1;
  static const double _workspaceContentMinWidth = 420;
  static const double _workspaceSidebarBreakpoint =
      _workspaceSidebarWidth +
      _workspaceDividerWidth +
      _workspaceContentMinWidth;
  static const double _tabletContentMaxWidth = 840;
  static const double _tabletQuickActionClearance = 56 + Grid.gutter;
  static const Duration _tabIconWeightDuration = Duration(milliseconds: 120);

  static const _phoneDestinations = [
    _HomeDestination(
      pageIndex: 0,
      icon: LucideIcons.house300,
      selectedIcon: LucideIcons.house500,
      label: 'Home',
    ),
    _HomeDestination(
      pageIndex: 1,
      icon: LucideIcons.inbox300,
      selectedIcon: LucideIcons.inbox500,
      label: 'Activity',
    ),
    _HomeDestination(
      pageIndex: 3,
      icon: LucideIcons.search300,
      selectedIcon: LucideIcons.search500,
      label: 'Search',
    ),
  ];

  static const _tabletDestinations = [
    _HomeDestination(
      pageIndex: 0,
      icon: LucideIcons.house300,
      selectedIcon: LucideIcons.house500,
      label: 'Home',
    ),
    _HomeDestination(
      pageIndex: 1,
      icon: LucideIcons.inbox300,
      selectedIcon: LucideIcons.inbox500,
      label: 'Activity',
    ),
    _HomeDestination(
      pageIndex: 2,
      icon: LucideIcons.bot,
      selectedIcon: LucideIcons.bot,
      label: 'Agents',
    ),
    _HomeDestination(
      pageIndex: 3,
      icon: LucideIcons.search300,
      selectedIcon: LucideIcons.search500,
      label: 'Search',
    ),
  ];

  static const _workspaceDestinations = [
    _HomeDestination(
      pageIndex: 1,
      icon: LucideIcons.inbox300,
      selectedIcon: LucideIcons.inbox500,
      label: 'Inbox',
    ),
    _HomeDestination(
      pageIndex: 2,
      icon: LucideIcons.bot,
      selectedIcon: LucideIcons.bot,
      label: 'Agents',
    ),
    _HomeDestination(
      pageIndex: 3,
      icon: LucideIcons.search300,
      selectedIcon: LucideIcons.search500,
      label: 'Search',
    ),
  ];

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final selectedPageIndex = useState(0);
    final selectedChannelId = useState<String?>(null);
    final channels = ref.watch(channelsProvider).asData?.value;
    Channel? selectedChannel;
    for (final channel in channels ?? const <Channel>[]) {
      if (channel.id != selectedChannelId.value) continue;
      selectedChannel = channel;
      break;
    }
    final windowSize = MediaQuery.sizeOf(context);
    final tabletWindow = windowSize.width >= HomePage._tabletBreakpoint;
    final systemBottomInset = MediaQuery.paddingOf(context).bottom;

    final pages = [
      ChannelsPage(settingsPageBuilder: settingsPageBuilder),
      const ActivityPage(),
      tabletWindow ? const AgentsPage() : const SizedBox.shrink(),
      const SearchPage(),
    ];

    void selectDestination(int pageIndex) {
      if (pageIndex == selectedPageIndex.value &&
          selectedChannelId.value == null) {
        return;
      }
      unawaited(HapticFeedback.selectionClick());
      selectedChannelId.value = null;
      selectedPageIndex.value = pageIndex;
    }

    Future<void> selectChannel(Channel channel) async {
      if (selectedChannelId.value == channel.id) return;
      unawaited(HapticFeedback.selectionClick());
      selectedChannelId.value = channel.id;
    }

    if (tabletWindow) {
      return Scaffold(
        backgroundColor: context.colors.surface,
        // Keep navigation and Home quick actions anchored above the keyboard.
        resizeToAvoidBottomInset: false,
        body: _TabletHomeShell(
          selectedPageIndex: selectedPageIndex.value,
          selectedChannel: selectedChannel,
          onDestinationSelected: selectDestination,
          onChannelSelected: selectChannel,
          settingsPageBuilder: settingsPageBuilder,
          railDestinations: _tabletDestinations,
          workspaceDestinations: _workspaceDestinations,
          pages: pages,
        ),
      );
    }

    final compactPageIndex =
        _phoneDestinations.any(
          (destination) => destination.pageIndex == selectedPageIndex.value,
        )
        ? selectedPageIndex.value
        : 0;
    final selectedDestinationIndex = _phoneDestinations.indexWhere(
      (destination) => destination.pageIndex == compactPageIndex,
    );
    final navigationBarWidth = _floatingTabBarWidth(
      windowSize.width,
      _phoneDestinations.length,
    );
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
            Positioned.fill(child: ColoredBox(color: context.colors.surface)),
            Positioned.fill(
              child: MediaQuery(
                data: _mediaQueryWithBottomClearance(
                  context,
                  HomePage._fabClearance,
                ),
                child: IndexedStack(index: compactPageIndex, children: pages),
              ),
            ),
            Align(
              alignment: Alignment.bottomCenter,
              child: IgnorePointer(
                child: MobileTabFooterBackdrop(
                  height: mobileTabFooterBackdropHeight(context),
                ),
              ),
            ),
            Positioned.fill(
              child: ChannelQuickActionsLauncher(
                visible: compactPageIndex == 0,
                placement: ChannelQuickActionsPlacement.besideBottomNavigation(
                  screenWidth: windowSize.width,
                  navigationBarHeight: HomePage._tabBarHeight,
                  navigationBarBottomGap: HomePage._tabBarBottomGap,
                  navigationBarWidth: navigationBarWidth,
                  systemBottomInset: systemBottomInset,
                  rightInset: Grid.sm,
                ),
              ),
            ),
          ],
        ),
      ),
      bottomNavigationBar: KeyedSubtree(
        key: const Key('mobile-navigation-bar'),
        child: _FloatingTabBar(
          selectedIndex: selectedDestinationIndex,
          onDestinationSelected: (index) =>
              selectDestination(_phoneDestinations[index].pageIndex),
          destinations: _phoneDestinations,
        ),
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

MediaQueryData _mediaQueryWithBottomClearance(
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

class _TabletHomeShell extends StatelessWidget {
  final int selectedPageIndex;
  final Channel? selectedChannel;
  final ValueChanged<int> onDestinationSelected;
  final Future<void> Function(Channel channel) onChannelSelected;
  final WidgetBuilder settingsPageBuilder;
  final List<_HomeDestination> railDestinations;
  final List<_HomeDestination> workspaceDestinations;
  final List<Widget> pages;

  const _TabletHomeShell({
    required this.selectedPageIndex,
    required this.selectedChannel,
    required this.onDestinationSelected,
    required this.onChannelSelected,
    required this.settingsPageBuilder,
    required this.railDestinations,
    required this.workspaceDestinations,
    required this.pages,
  });

  @override
  Widget build(BuildContext context) {
    final colorScheme = context.colors;
    final windowWidth = MediaQuery.sizeOf(context).width;
    final showWorkspaceSidebar =
        windowWidth >= HomePage._workspaceSidebarBreakpoint;
    final selectedDestinationIndex = railDestinations.indexWhere(
      (destination) => destination.pageIndex == selectedPageIndex,
    );
    final workspacePageIndex = selectedPageIndex == 0 ? 1 : selectedPageIndex;
    final workspaceDestinationIndex = workspaceDestinations.indexWhere(
      (destination) => destination.pageIndex == workspacePageIndex,
    );

    return Row(
      children: [
        if (showWorkspaceSidebar)
          _TabletWorkspaceSidebar(
            selectedPageIndex: workspacePageIndex,
            selectedChannelId: selectedChannel?.id,
            onDestinationSelected: onDestinationSelected,
            onChannelSelected: onChannelSelected,
            settingsPageBuilder: settingsPageBuilder,
            destinations: workspaceDestinations,
          )
        else
          _TabletNavigationRail(
            selectedIndex: selectedDestinationIndex,
            onDestinationSelected: (index) =>
                onDestinationSelected(railDestinations[index].pageIndex),
            destinations: railDestinations,
          ),
        VerticalDivider(
          width: HomePage._workspaceDividerWidth,
          thickness: HomePage._workspaceDividerWidth,
          color: colorScheme.outlineVariant.withValues(alpha: 0.45),
        ),
        Expanded(
          child: Center(
            child: ConstrainedBox(
              constraints: const BoxConstraints(
                maxWidth: HomePage._tabletContentMaxWidth,
              ),
              child: LayoutBuilder(
                builder: (context, constraints) {
                  final paneSize = constraints.biggest;
                  final systemBottomInset = MediaQuery.paddingOf(
                    context,
                  ).bottom;
                  final paneMediaQuery = _mediaQueryWithBottomClearance(
                    context,
                    HomePage._tabletQuickActionClearance,
                  ).copyWith(size: paneSize);

                  return MediaQuery(
                    data: paneMediaQuery,
                    child: SizedBox.expand(
                      key: const Key('tablet-page-content'),
                      child: Stack(
                        fit: StackFit.expand,
                        children: [
                          if (showWorkspaceSidebar)
                            IndexedStack(
                              index: workspaceDestinationIndex,
                              children: const [
                                ActivityPage(title: 'Inbox'),
                                AgentsPage(),
                                SearchPage(),
                              ],
                            )
                          else
                            IndexedStack(
                              index: selectedPageIndex,
                              children: pages,
                            ),
                          if (selectedChannel case final channel?)
                            Positioned.fill(
                              child: ChannelDetailPage(
                                key: ValueKey('tablet-channel-${channel.id}'),
                                channel: channel,
                              ),
                            ),
                          Positioned.fill(
                            child: ChannelQuickActionsLauncher(
                              visible:
                                  selectedChannel == null &&
                                  (showWorkspaceSidebar
                                      ? workspacePageIndex == 1
                                      : selectedPageIndex == 0),
                              placement:
                                  ChannelQuickActionsPlacement.bottomTrailing(
                                    systemBottomInset: systemBottomInset,
                                  ),
                            ),
                          ),
                        ],
                      ),
                    ),
                  );
                },
              ),
            ),
          ),
        ),
      ],
    );
  }
}

class _TabletNavigationRail extends StatelessWidget {
  const _TabletNavigationRail({
    required this.selectedIndex,
    required this.onDestinationSelected,
    required this.destinations,
  });

  final int selectedIndex;
  final ValueChanged<int> onDestinationSelected;
  final List<_HomeDestination> destinations;

  @override
  Widget build(BuildContext context) {
    final colorScheme = context.colors;

    return ColoredBox(
      color: colorScheme.surfaceContainerLow,
      child: SafeArea(
        right: false,
        child: NavigationRail(
          key: const Key('tablet-navigation-rail'),
          selectedIndex: selectedIndex,
          onDestinationSelected: onDestinationSelected,
          minWidth: HomePage._tabletRailWidth,
          groupAlignment: -0.72,
          labelType: NavigationRailLabelType.all,
          backgroundColor: colorScheme.surfaceContainerLow,
          indicatorColor: colorScheme.primaryContainer,
          selectedIconTheme: IconThemeData(
            color: colorScheme.onPrimaryContainer,
            size: HomePage._tabIconSize,
          ),
          unselectedIconTheme: IconThemeData(
            color: colorScheme.onSurfaceVariant,
            size: HomePage._tabIconSize,
          ),
          selectedLabelTextStyle: context.textTheme.labelMedium?.copyWith(
            color: colorScheme.onSurface,
            fontWeight: FontWeight.w600,
          ),
          unselectedLabelTextStyle: context.textTheme.labelMedium?.copyWith(
            color: colorScheme.onSurfaceVariant,
            fontWeight: FontWeight.w500,
          ),
          destinations: [
            for (final destination in destinations)
              NavigationRailDestination(
                icon: Icon(destination.icon),
                selectedIcon: Icon(destination.selectedIcon),
                label: Text(destination.label),
              ),
          ],
        ),
      ),
    );
  }
}

class _HomeDestination {
  final int pageIndex;
  final IconData icon;
  final IconData selectedIcon;
  final String label;

  const _HomeDestination({
    required this.pageIndex,
    required this.icon,
    required this.selectedIcon,
    required this.label,
  });
}

class _FloatingTabBar extends StatelessWidget {
  final int selectedIndex;
  final ValueChanged<int> onDestinationSelected;
  final List<_HomeDestination> destinations;

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
  final _HomeDestination destination;
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
