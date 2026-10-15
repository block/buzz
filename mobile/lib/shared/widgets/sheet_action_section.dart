import 'package:flutter/material.dart';

import '../theme/theme.dart';
import 'app_list_card.dart';

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
      minLeadingWidth: 22,
      horizontalTitleGap: Grid.xs,
      child: AppListCard(
        horizontalPadding: horizontalPadding,
        label: label,
        dividerIndent: dividerIndent,
        children: children,
      ),
    );
  }
}
