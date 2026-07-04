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

  const FilterChipBar({
    super.key,
    required this.items,
    required this.selected,
    required this.onSelected,
  });

  @override
  Widget build(BuildContext context) {
    return SingleChildScrollView(
      scrollDirection: Axis.horizontal,
      padding: const EdgeInsets.symmetric(
        horizontal: Grid.gutter,
        vertical: Grid.xxs,
      ),
      child: Row(
        children: [
          for (final item in items) ...[
            if (item != items.first) const SizedBox(width: Grid.xxs),
            _chip(context, item),
          ],
        ],
      ),
    );
  }

  Widget _chip(BuildContext context, FilterChipItem<T> item) {
    final isSelected = selected == item.id;
    final text = item.count != null
        ? '${item.label} (${item.count})'
        : item.label;
    final fg = isSelected
        ? context.colors.onPrimary
        : context.colors.onSurfaceVariant;
    final labelStyle = context.textTheme.labelSmall?.copyWith(color: fg);
    return FilterChip(
      selected: isSelected,
      showCheckmark: false,
      label: item.icon != null
          ? Row(
              mainAxisSize: MainAxisSize.min,
              children: [
                Icon(item.icon, size: 14, color: fg),
                const SizedBox(width: Grid.half),
                Text(text, style: labelStyle),
              ],
            )
          : Text(text, style: labelStyle),
      onSelected: (_) => onSelected(item.id),
      visualDensity: VisualDensity.compact,
      materialTapTargetSize: MaterialTapTargetSize.shrinkWrap,
    );
  }
}
