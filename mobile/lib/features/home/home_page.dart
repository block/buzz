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
  static const double _tabBarHorizontalMargin = Grid.sm;
  static const double _fabClearance = _tabBarHeight + _tabBarBottomGap;

  static const _destinations = [
    _HomeDestination(
      icon: LucideIcons.house,
      selectedIcon: LucideIcons.house,
      label: 'Home',
    ),
    _HomeDestination(
      icon: LucideIcons.bell,
      selectedIcon: LucideIcons.bell,
      label: 'Activity',
    ),
    _HomeDestination(
      icon: LucideIcons.search,
      selectedIcon: LucideIcons.search,
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
    return SafeArea(
      minimum: const EdgeInsets.fromLTRB(
        HomePage._tabBarHorizontalMargin,
        0,
        HomePage._tabBarHorizontalMargin,
        HomePage._tabBarBottomGap,
      ),
      child: Align(
        alignment: Alignment.bottomCenter,
        child: ConstrainedBox(
          constraints: const BoxConstraints(maxWidth: 336),
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
                          child: Row(
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
    final foregroundColor = selected
        ? colorScheme.onPrimary
        : colorScheme.onSurfaceVariant;

    return Padding(
      padding: const EdgeInsets.symmetric(horizontal: Grid.quarter),
      child: Material(
        color: selected
            ? colorScheme.primary.withValues(alpha: 0.94)
            : Colors.transparent,
        borderRadius: BorderRadius.circular(HomePage._selectedTabRadius),
        clipBehavior: Clip.antiAlias,
        child: InkWell(
          onTap: onTap,
          borderRadius: BorderRadius.circular(HomePage._selectedTabRadius),
          child: AnimatedContainer(
            duration: const Duration(milliseconds: 180),
            curve: Curves.easeOutCubic,
            padding: const EdgeInsets.symmetric(
              horizontal: Grid.xxs,
              vertical: Grid.xxs,
            ),
            child: Column(
              mainAxisAlignment: MainAxisAlignment.center,
              mainAxisSize: MainAxisSize.min,
              children: [
                Icon(
                  selected ? destination.selectedIcon : destination.icon,
                  color: foregroundColor,
                  size: 20,
                ),
                const SizedBox(height: 1),
                Text(
                  destination.label,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: textStyle?.copyWith(
                    color: foregroundColor,
                    fontSize: 10.5,
                    fontWeight: selected ? FontWeight.w700 : FontWeight.w600,
                    letterSpacing: 0.05,
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
