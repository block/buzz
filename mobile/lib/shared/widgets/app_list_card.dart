import 'package:flutter/material.dart';

import '../theme/theme.dart';
import 'app_list_inset.dart';

/// A group of list rows in a rounded container, with an optional [label] above
/// it. Rows inside are hairline-separated and inset to the card rather than the
/// page, via [AppListInset].
class AppListCard extends StatelessWidget {
  const AppListCard({
    super.key,
    this.label,
    this.dividerIndent,
    this.verticalPadding = Grid.xxs,
    required this.children,
  });

  /// Rendered above the card in sentence case, as written — no uppercasing.
  final String? label;

  /// Separator inset from the card edge. Defaults to the standard row label
  /// column, clearing a leading icon. Icon-free cards can pass [_inset] so
  /// separators align with their row content on both sides.
  final double? dividerIndent;

  /// Outer padding above and below this section.
  ///
  /// Adjacent cards contribute this padding from both sides. For example,
  /// passing [Grid.twelve] creates a 24dp rhythm between grouped surfaces.
  final double verticalPadding;

  final List<Widget> children;

  static const _inset = Grid.xs;

  /// Separators start at the label column, clearing the leading icon.
  static const _dividerIndent = _inset + _iconColumnWidth;
  static const _iconColumnWidth = 22.0 + Grid.xs;

  @override
  Widget build(BuildContext context) {
    final separated = <Widget>[];
    for (var index = 0; index < children.length; index++) {
      if (index > 0) {
        separated.add(
          Divider(
            height: 1,
            thickness: 1,
            indent: dividerIndent ?? _dividerIndent,
            endIndent: _inset,
            // The scheme's own border tokens are derived from the page surface,
            // which lands them within a few levels of the card fill — invisible.
            // Tinting with the text color instead keeps the hairline readable on
            // the card in both brightnesses.
            color: context.colors.onSurface.withValues(alpha: 0.12),
          ),
        );
      }
      separated.add(children[index]);
    }

    return Padding(
      padding: EdgeInsets.fromLTRB(
        Grid.gutter,
        verticalPadding,
        Grid.gutter,
        verticalPadding,
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          if (label != null)
            Padding(
              padding: const EdgeInsets.only(left: Grid.half, bottom: Grid.xxs),
              child: Text(
                label!,
                style: context.textTheme.labelMedium?.copyWith(
                  color: context.colors.onSurfaceVariant,
                  fontWeight: FontWeight.w600,
                ),
              ),
            ),
          Material(
            // The softest step on the elevation ramp — a rung below the home tab
            // bar's active pill (primaryContainer, see PR #2810), since a
            // full-width card at the pill's contrast reads heavier than the pill
            // does. The dividers carry the group structure, so the fill only has
            // to separate the card from the page.
            color: context.colors.surfaceContainerHighest,
            borderRadius: BorderRadius.circular(Radii.container),
            // Keeps row ripples inside the rounded corners.
            clipBehavior: Clip.antiAlias,
            child: AppListInset(
              horizontal: _inset,
              child: Column(children: separated),
            ),
          ),
        ],
      ),
    );
  }
}
