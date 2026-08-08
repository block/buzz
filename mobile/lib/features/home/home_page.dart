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
import '../channels/channels_page.dart';
import '../search/search_page.dart';

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
    final settingsTransitionProgress = useValueNotifier(0.0);
    final reducedMotion = MediaQuery.of(context).disableAnimations;
    final tabContentTransitionProgress = reducedMotion
        ? 1.0
        : _tabContentTransitionCurve.transform(tabContentTransitionValue);
    final systemBottomInset = MediaQuery.paddingOf(context).bottom;
    final navigationBarWidth = _floatingTabBarWidth(
      MediaQuery.sizeOf(context).width,
      _destinations.length,
    );

    final pages = [
      ChannelsPage(
        settingsPageBuilder: settingsPageBuilder,
        tabReselection: homeReselection,
        onSettingsTransitionProgress: (progress) {
          if (settingsTransitionProgress.value != progress) {
            settingsTransitionProgress.value = progress;
          }
        },
      ),
      ActivityPage(tabReselection: activityReselection),
      SearchPage(tabReselection: searchReselection),
    ];

    final settingsTransitionGradient = tabIndex.value == 0
        ? context.appColors.topSectionGradient
        : null;

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
          valueListenable: settingsTransitionProgress,
          child: Scaffold(
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
                    child: ColoredBox(color: context.colors.surface),
                  ),
                  Positioned.fill(
                    child: MediaQuery(
                      data: _mediaQueryWithFloatingTabBarClearance(
                        context,
                        HomePage._fabClearance,
                      ),
                      child: DirectionalTransitionScope(
                        horizontalOffset:
                            tabContentTransitionDirection.value *
                            _tabContentTransitionDistance *
                            (1 - tabContentTransitionProgress),
                        opacity: tabContentTransitionProgress,
                        child: ClipRect(
                          child: IndexedStack(
                            index: tabIndex.value,
                            children: pages,
                          ),
                        ),
                      ),
                    ),
                  ),
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
                    child: ChannelQuickActionsLauncher(
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
            bottomNavigationBar: _FloatingTabBar(
              selectedIndex: tabIndex.value,
              hasUnreadInbox: hasUnreadInbox,
              onDestinationSelected: (i) {
                if (i == tabIndex.value) {
                  switch (i) {
                    case 0:
                      homeReselection.value++;
                    case 1:
                      activityReselection.value++;
                    case 2:
                      searchReselection.value++;
                  }
                  return;
                }
                tabContentTransitionDirection.value = i > tabIndex.value
                    ? 1
                    : -1;
                unawaited(HapticFeedback.selectionClick());
                tabIndex.value = i;
                if (reducedMotion) {
                  tabContentTransitionController.value = 1;
                } else {
                  unawaited(tabContentTransitionController.forward(from: 0));
                }
              },
              destinations: _destinations,
            ),
          ),
          builder: (context, progress, child) {
            final curvedProgress = reducedMotion
                ? 0.0
                : Curves.easeOutCubic.transform(progress);
            return Opacity(
              key: const ValueKey('home-settings-transition-opacity'),
              opacity: 1 - curvedProgress,
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
