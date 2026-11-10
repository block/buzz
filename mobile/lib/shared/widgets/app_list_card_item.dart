import 'package:flutter/material.dart';

import '../theme/theme.dart';

/// One lazily built row of a rounded list section.
///
/// Use inside a builder list with no inter-row spacing. Only the first and
/// last rows round their outer corners, and separators stay inside the group.
/// The parent list owns the section's outer gutter and vertical spacing.
class AppListCardItem extends StatelessWidget {
  const AppListCardItem({
    super.key,
    required this.index,
    required this.itemCount,
    required this.child,
    this.dividerIndent = Grid.xs + 22 + Grid.xs,
  });

  final int index;
  final int itemCount;
  final Widget child;
  final double dividerIndent;

  @override
  Widget build(BuildContext context) {
    const corner = Radius.circular(Radii.container);
    return Material(
      color: context.colors.surfaceContainerHighest,
      borderRadius: BorderRadius.vertical(
        top: index == 0 ? corner : Radius.zero,
        bottom: index == itemCount - 1 ? corner : Radius.zero,
      ),
      clipBehavior: Clip.antiAlias,
      child: ListTileTheme.merge(
        contentPadding: const EdgeInsets.symmetric(horizontal: Grid.xs),
        minLeadingWidth: 22,
        horizontalTitleGap: Grid.xs,
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            child,
            if (index < itemCount - 1)
              Divider(
                height: 1,
                thickness: 1,
                indent: dividerIndent,
                endIndent: Grid.xs,
                color: context.colors.onSurface.withValues(alpha: 0.12),
              ),
          ],
        ),
      ),
    );
  }
}
