import 'package:flutter/material.dart';

import '../theme/theme.dart';
import 'app_list_card.dart';

export 'app_list_inset.dart' show AppListRowIcon;

/// A sheet action group using the same rounded card as settings and lists.
/// The caller owns the sheet gutter; rows share the card's inset and dividers.
class SheetActionSection extends StatelessWidget {
  const SheetActionSection({
    super.key,
    required this.children,
    this.horizontalPadding = 0,
    this.label,
    this.dividerIndent,
  });

  final List<Widget> children;

  final double horizontalPadding;
  final String? label;
  final double? dividerIndent;

  @override
  Widget build(BuildContext context) {
    if (children.isEmpty) return const SizedBox.shrink();
    return ListTileTheme.merge(
      contentPadding: const EdgeInsets.symmetric(horizontal: Grid.xs),
      minLeadingWidth: Grid.md,
      minVerticalPadding: Grid.xs,
      minTileHeight: Grid.xs * 2 + 22,
      titleAlignment: ListTileTitleAlignment.center,
      titleTextStyle: sheetActionTextStyle.copyWith(
        color: context.colors.onSurface,
      ),
      subtitleTextStyle: context.textTheme.bodySmall?.copyWith(
        color: context.colors.onSurfaceVariant,
      ),
      iconColor: context.colors.onSurfaceVariant,
      horizontalTitleGap: Grid.xs,
      child: AppListCard.sheet(
        horizontalPadding: horizontalPadding,
        label: label,
        dividerIndent: dividerIndent,
        children: children,
      ),
    );
  }
}
