import 'package:flutter/material.dart';

import '../../shared/theme/theme.dart';

class PulseTabSpec {
  final String id;
  final String label;
  final IconData icon;

  const PulseTabSpec({
    required this.id,
    required this.label,
    required this.icon,
  });
}

class PulseTabBar extends StatelessWidget {
  final List<PulseTabSpec> tabs;
  final String selected;
  final ValueChanged<String> onSelected;

  const PulseTabBar({
    super.key,
    required this.tabs,
    required this.selected,
    required this.onSelected,
  });

  @override
  Widget build(BuildContext context) {
    return SingleChildScrollView(
      scrollDirection: Axis.horizontal,
      padding: const EdgeInsets.fromLTRB(
        Grid.xs,
        Grid.twelve,
        Grid.xs,
        Grid.xxs,
      ),
      child: Row(
        children: [
          for (final tab in tabs) ...[
            if (tab != tabs.first) const SizedBox(width: Grid.xxs),
            GestureDetector(
              onTap: () => onSelected(tab.id),
              child: AnimatedContainer(
                duration: const Duration(milliseconds: 160),
                padding: const EdgeInsets.symmetric(
                  horizontal: Grid.twelve,
                  vertical: Grid.half + 2,
                ),
                decoration: BoxDecoration(
                  color: selected == tab.id
                      ? context.colors.primary
                      : context.colors.surfaceContainerHighest,
                  borderRadius: BorderRadius.circular(Radii.lg),
                  border: selected == tab.id
                      ? null
                      : Border.all(color: context.colors.outlineVariant),
                ),
                child: Row(
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    Icon(
                      tab.icon,
                      size: 14,
                      color: selected == tab.id
                          ? context.colors.onPrimary
                          : context.colors.onSurfaceVariant,
                    ),
                    const SizedBox(width: Grid.half),
                    Text(
                      tab.label,
                      style: context.textTheme.labelMedium?.copyWith(
                        color: selected == tab.id
                            ? context.colors.onPrimary
                            : context.colors.onSurfaceVariant,
                        fontWeight: FontWeight.w600,
                      ),
                    ),
                  ],
                ),
              ),
            ),
          ],
        ],
      ),
    );
  }
}
