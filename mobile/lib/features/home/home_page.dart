import 'dart:ui';

import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';

import '../../shared/theme/grid.dart';
import '../activity/activity_page.dart';
import '../channels/channels_page.dart';
import '../search/search_page.dart';

class HomePage extends HookConsumerWidget {
  const HomePage({super.key});

  static const double _tabBarHeight = 60;
  static const double _tabBarRadius = _tabBarHeight / 2;
  static const double _tabBarInnerInset = 5;
  static const double _selectedTabRadius =
      (_tabBarHeight - (_tabBarInnerInset * 2)) / 2;
  static const double _tabBarBottomGap = Grid.twelve;
  static const double _tabBarHorizontalMargin = Grid.gutter;
  static const double _fabClearance = _tabBarHeight + _tabBarBottomGap;
  static const Duration _tabIconWeightDuration = Duration(milliseconds: 120);

  static const _destinations = [
    _HomeDestination(
      icon: LucideIcons.house300,
      selectedIcon: LucideIcons.house500,
      label: 'Home',
    ),
    _HomeDestination(
      icon: LucideIcons.bell300,
      selectedIcon: LucideIcons.bell400,
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

    const pages = [ChannelsPage(), ActivityPage(), SearchPage()];

    return Scaffold(
      extendBody: true,
      body: MediaQuery(
        data: _mediaQueryWithFloatingTabBarClearance(
          context,
          HomePage._fabClearance,
        ),
        child: IndexedStack(index: tabIndex.value, children: pages),
      ),
      bottomNavigationBar: _FloatingTabBar(
        selectedIndex: tabIndex.value,
        onDestinationSelected: (i) => tabIndex.value = i,
        destinations: _destinations,
      ),
    );
  }
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
  final ValueChanged<int> onDestinationSelected;
  final List<_HomeDestination> destinations;

  const _FloatingTabBar({
    required this.selectedIndex,
    required this.onDestinationSelected,
    required this.destinations,
  });

  @override
  Widget build(BuildContext context) {
    final colorScheme = Theme.of(context).colorScheme;
    final isDark = Theme.of(context).brightness == Brightness.dark;
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

    return SafeArea(
      minimum: const EdgeInsets.fromLTRB(
        HomePage._tabBarHorizontalMargin,
        0,
        HomePage._tabBarHorizontalMargin,
        HomePage._tabBarBottomGap,
      ),
      child: Align(
        alignment: Alignment.bottomCenter,
        child: SizedBox(
          width: double.infinity,
          child: DecoratedBox(
            decoration: BoxDecoration(
              borderRadius: BorderRadius.circular(HomePage._tabBarRadius),
              boxShadow: [
                BoxShadow(
                  color: colorScheme.shadow.withValues(alpha: 0.18),
                  blurRadius: 28,
                  offset: const Offset(0, 12),
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
                        : null,
                    border: Border.all(
                      color: colorScheme.outlineVariant.withValues(
                        alpha: isDark ? 0.20 : 0.38,
                      ),
                    ),
                    gradient: isDark
                        ? null
                        : LinearGradient(
                            begin: Alignment.topCenter,
                            end: Alignment.bottomCenter,
                            colors: [
                              colorScheme.surface.withValues(alpha: 0.90),
                              colorScheme.surfaceContainerHighest.withValues(
                                alpha: 0.78,
                              ),
                            ],
                          ),
                  ),
                  child: Stack(
                    children: [
                      if (!isDark)
                        Positioned.fill(
                          child: DecoratedBox(
                            decoration: BoxDecoration(
                              gradient: LinearGradient(
                                begin: Alignment.topCenter,
                                end: Alignment.center,
                                colors: [
                                  Colors.white.withValues(alpha: 0.22),
                                  Colors.white.withValues(alpha: 0.02),
                                ],
                              ),
                            ),
                          ),
                        ),
                      Padding(
                        padding: const EdgeInsets.all(
                          HomePage._tabBarInnerInset,
                        ),
                        child: SizedBox(
                          height:
                              HomePage._tabBarHeight -
                              (HomePage._tabBarInnerInset * 2),
                          child: Stack(
                            children: [
                              AnimatedAlign(
                                alignment: selectedAlignment,
                                duration: const Duration(milliseconds: 180),
                                curve: Curves.easeOutCubic,
                                child: FractionallySizedBox(
                                  widthFactor: 1 / destinationCount,
                                  heightFactor: 1,
                                  child: Padding(
                                    padding: const EdgeInsets.symmetric(
                                      horizontal: Grid.quarter,
                                    ),
                                    child: DecoratedBox(
                                      decoration: BoxDecoration(
                                        color: colorScheme.secondaryContainer,
                                        borderRadius: BorderRadius.circular(
                                          HomePage._selectedTabRadius,
                                        ),
                                      ),
                                    ),
                                  ),
                                ),
                              ),
                              Row(
                                children: [
                                  for (var i = 0; i < destinations.length; i++)
                                    Expanded(
                                      child: _FloatingTabDestination(
                                        destination: destinations[i],
                                        selected: i == selectedIndex,
                                        onTap: () => onDestinationSelected(i),
                                      ),
                                    ),
                                ],
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
    final colorScheme = Theme.of(context).colorScheme;
    final textStyle = Theme.of(context).textTheme.labelSmall;
    final reducedMotion = MediaQuery.of(context).disableAnimations;
    final foregroundColor = selected
        ? colorScheme.onSecondaryContainer
        : colorScheme.onSurfaceVariant;
    final icon = selected ? destination.selectedIcon : destination.icon;

    return Padding(
      padding: const EdgeInsets.symmetric(horizontal: Grid.quarter),
      child: Material(
        color: Colors.transparent,
        borderRadius: BorderRadius.circular(HomePage._selectedTabRadius),
        clipBehavior: Clip.antiAlias,
        child: InkWell(
          onTap: onTap,
          splashFactory: NoSplash.splashFactory,
          overlayColor: const WidgetStatePropertyAll<Color>(Colors.transparent),
          borderRadius: BorderRadius.circular(HomePage._selectedTabRadius),
          child: Padding(
            padding: const EdgeInsets.symmetric(
              horizontal: Grid.xxs,
              vertical: Grid.xxs,
            ),
            child: Column(
              mainAxisAlignment: MainAxisAlignment.center,
              mainAxisSize: MainAxisSize.min,
              children: [
                AnimatedSwitcher(
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
                    size: 20,
                  ),
                ),
                const SizedBox(height: 1),
                Text(
                  destination.label,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: textStyle?.copyWith(
                    color: foregroundColor,
                    fontSize: 10.5,
                    fontWeight: selected ? FontWeight.w700 : FontWeight.w400,
                    height: 1.15,
                    letterSpacing: 0,
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
