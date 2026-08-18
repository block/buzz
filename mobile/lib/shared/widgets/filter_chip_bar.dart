import 'package:flutter/material.dart';

import '../theme/theme.dart';

/// A single entry in a [FilterChipBar].
class FilterChipItem<T> {
  /// Value identifying this chip; compared against the bar's `selected`.
  final T id;

  /// Visible label.
  final String label;

  /// Optional leading icon.
  final IconData? icon;

  /// Optional count appended to the label as " (n)".
  final int? count;

  const FilterChipItem({
    required this.id,
    required this.label,
    this.icon,
    this.count,
  });
}

/// A horizontally-scrolling row of Material [FilterChip]s.
///
/// This is the single shared filter/segment selector used across Pulse,
/// Search, and Activity. It relies on the app's `chipTheme` for styling so
/// every screen looks identical — do not hand-roll chip rows elsewhere.
class FilterChipBar<T> extends StatelessWidget {
  final List<FilterChipItem<T>> items;
  final T selected;
  final ValueChanged<T> onSelected;

  /// When enabled, every chip shares the width rather than scrolling.
  final bool expandItems;

  /// Material density applied to each chip.
  final VisualDensity visualDensity;

  /// Vertical padding around the content inside each chip.
  final double chipVerticalPadding;

  /// Vertical spacing around the chip row.
  final double barVerticalPadding;

  const FilterChipBar({
    super.key,
    required this.items,
    required this.selected,
    required this.onSelected,
    this.expandItems = false,
    this.visualDensity = VisualDensity.compact,
    this.chipVerticalPadding = Grid.quarter,
    this.barVerticalPadding = Grid.xxs,
  });

  @override
  Widget build(BuildContext context) {
    final chips = [
      for (var index = 0; index < items.length; index++) ...[
        if (index > 0) const SizedBox(width: Grid.xxs),
        if (expandItems)
          Expanded(child: _chip(context, items[index], fillWidth: true))
        else
          _chip(context, items[index]),
      ],
    ];

    if (expandItems) {
      return Padding(
        padding: EdgeInsets.symmetric(
          horizontal: Grid.gutter,
          vertical: barVerticalPadding,
        ),
        child: Row(children: chips),
      );
    }

    return SingleChildScrollView(
      scrollDirection: Axis.horizontal,
      padding: EdgeInsets.symmetric(
        horizontal: Grid.gutter,
        vertical: barVerticalPadding,
      ),
      child: Row(children: chips),
    );
  }

  Widget _chip(
    BuildContext context,
    FilterChipItem<T> item, {
    bool fillWidth = false,
  }) {
    final isSelected = selected == item.id;
    final text = item.count != null
        ? '${item.label} (${item.count})'
        : item.label;
    final fg = isSelected
        ? context.colors.onPrimary
        : context.colors.onSurfaceVariant;
    final labelStyle = filterChipTextStyle.copyWith(
      color: fg,
      fontWeight: isSelected ? FontWeight.w500 : FontWeight.w400,
    );
    final label = item.icon != null
        ? Row(
            mainAxisSize: fillWidth ? MainAxisSize.max : MainAxisSize.min,
            mainAxisAlignment: MainAxisAlignment.center,
            children: [
              Icon(item.icon, size: 14, color: fg),
              const SizedBox(width: Grid.half),
              if (fillWidth)
                Flexible(
                  child: Text(
                    text,
                    textAlign: TextAlign.center,
                    style: labelStyle,
                  ),
                )
              else
                Text(text, style: labelStyle),
            ],
          )
        : Text(
            text,
            textAlign: fillWidth ? TextAlign.center : TextAlign.start,
            style: labelStyle,
          );
    final centeredLabel = Align(
      alignment: Alignment.center,
      widthFactor: fillWidth ? null : 1,
      heightFactor: 1,
      child: label,
    );
    final chip = FilterChip(
      selected: isSelected,
      showCheckmark: false,
      shape: RoundedRectangleBorder(
        borderRadius: BorderRadius.circular(Radii.lg),
      ),
      side: BorderSide.none,
      label: fillWidth
          ? SizedBox(width: double.infinity, child: centeredLabel)
          : centeredLabel,
      labelPadding: EdgeInsets.zero,
      onSelected: (_) => onSelected(item.id),
      padding: EdgeInsets.symmetric(
        horizontal: fillWidth ? Grid.quarter : Grid.twelve,
        vertical: chipVerticalPadding,
      ),
      visualDensity: visualDensity,
      materialTapTargetSize: MaterialTapTargetSize.shrinkWrap,
    );

    return fillWidth ? SizedBox(width: double.infinity, child: chip) : chip;
  }
}
