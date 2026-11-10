import 'dart:math' as math;

import 'package:flutter/material.dart';
import 'package:flutter/foundation.dart';
import 'ios_glass_navigation_button.dart';

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

  /// Keeps unread activity visible when its inventory is filtered out.
  final bool hasUnread;

  const FilterChipItem({
    required this.id,
    required this.label,
    this.icon,
    this.count,
    this.hasUnread = false,
  });
}

/// A horizontally-scrolling row of Material [FilterChip]s.
///
/// This is the single shared filter/segment selector used across Search and
/// Activity. It relies on the app's `chipTheme` for styling so every screen
/// looks identical — do not hand-roll chip rows elsewhere.
class FilterChipBar<T> extends StatelessWidget {
  final List<FilterChipItem<T>> items;
  final T selected;
  final ValueChanged<T> onSelected;

  /// When enabled, every chip shares the width rather than scrolling.
  final bool expandItems;

  /// Uses native glass capsules on iOS for navigation-style filters.
  final bool useNativeGlass;

  /// Uses the native iOS label widths for Material capsules too.
  final bool matchNativeWidths;

  /// Optional outline for Material capsules.
  final BorderSide side;

  /// Optical adjustment for the visible label within its centered layout box.
  final Offset labelOffset;

  /// Minimum visible capsule height; the hit target may be larger.
  final double minControlHeight;

  /// Material density applied to each chip.
  final VisualDensity visualDensity;

  /// Vertical padding around the content inside each chip.
  final double chipVerticalPadding;

  /// Vertical spacing around the chip row.
  final double barVerticalPadding;

  /// Horizontal spacing around the chip row.
  final double horizontalPadding;

  const FilterChipBar({
    super.key,
    required this.items,
    required this.selected,
    required this.onSelected,
    this.expandItems = false,
    this.useNativeGlass = false,
    this.matchNativeWidths = false,
    this.side = BorderSide.none,
    this.labelOffset = Offset.zero,
    this.minControlHeight = 0,
    this.visualDensity = VisualDensity.compact,
    this.chipVerticalPadding = Grid.quarter,
    this.barVerticalPadding = Grid.xxs,
    this.horizontalPadding = Grid.gutter,
  });

  @override
  Widget build(BuildContext context) {
    final widths = [
      for (final item in items)
        (TextPainter(
              text: TextSpan(
                text: item.label,
                style: filterChipTextStyle.copyWith(
                  fontWeight: FontWeight.w600,
                ),
              ),
              textScaler: MediaQuery.textScalerOf(context),
              textDirection: Directionality.of(context),
            )..layout()).width +
            32,
    ];
    if (useNativeGlass && defaultTargetPlatform == TargetPlatform.iOS) {
      final height = math.max(
        MediaQuery.textScalerOf(context).scale(16) * 1.4 + 16,
        math.max(48.0, minControlHeight),
      );
      return LayoutBuilder(
        builder: (context, constraints) {
          final available = constraints.maxWidth - horizontalPadding * 2;
          final required =
              widths.fold(0.0, (sum, width) => sum + width) +
              Grid.xxs * (items.length - 1);
          final extra = expandItems && available > required
              ? (available - required) / items.length
              : 0.0;
          return SingleChildScrollView(
            scrollDirection: Axis.horizontal,
            clipBehavior: Clip.none,
            padding: EdgeInsets.symmetric(
              horizontal: horizontalPadding,
              vertical: barVerticalPadding,
            ),
            child: Row(
              children: [
                for (var index = 0; index < items.length; index++) ...[
                  if (index > 0) const SizedBox(width: Grid.xxs),
                  Badge(
                    isLabelVisible: items[index].hasUnread,
                    alignment: Alignment.topRight,
                    offset: const Offset(-6, 6),
                    child: IosGlassNavigationButton(
                      icon: IosGlassNavigationIcon.more,
                      label: items[index].label,
                      semanticLabel: items[index].hasUnread
                          ? '${items[index].label}, unread'
                          : items[index].label,
                      onPressed: () => onSelected(items[index].id),
                      isSelected: selected == items[index].id,
                      width: widths[index] + extra,
                      height: height,
                      controlSize: math
                          .max(height - 8, minControlHeight)
                          .toDouble(),
                      fillWidth: true,
                      foregroundColor: context.colors.primary,
                    ),
                  ),
                ],
              ],
            ),
          );
        },
      );
    }
    final chips = [
      for (var index = 0; index < items.length; index++) ...[
        if (index > 0) const SizedBox(width: Grid.xxs),
        if (expandItems)
          Expanded(child: _chip(context, items[index], fillWidth: true))
        else if (matchNativeWidths)
          SizedBox(
            width: widths[index],
            child: _chip(context, items[index], fillWidth: true),
          )
        else
          _chip(context, items[index]),
      ],
    ];

    if (expandItems) {
      return Padding(
        padding: EdgeInsets.symmetric(
          horizontal: horizontalPadding,
          vertical: barVerticalPadding,
        ),
        child: Row(children: chips),
      );
    }

    return SingleChildScrollView(
      scrollDirection: Axis.horizontal,
      padding: EdgeInsets.symmetric(
        horizontal: horizontalPadding,
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
      leadingDistribution: defaultTargetPlatform == TargetPlatform.android
          ? TextLeadingDistribution.even
          : null,
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
    final sizedLabel = minControlHeight > 0
        ? ConstrainedBox(
            constraints: BoxConstraints(
              minHeight: math.max(
                0,
                minControlHeight - chipVerticalPadding * 2,
              ),
            ),
            child: Center(
              widthFactor: 1,
              heightFactor: 1,
              child: Transform.translate(offset: labelOffset, child: label),
            ),
          )
        : label;
    final centeredLabel = Align(
      alignment: Alignment.center,
      widthFactor: fillWidth ? null : 1,
      heightFactor: 1,
      child: sizedLabel,
    );
    final chip = FilterChip(
      selected: isSelected,
      showCheckmark: false,
      shape: useNativeGlass || defaultTargetPlatform == TargetPlatform.android
          ? const StadiumBorder()
          : RoundedRectangleBorder(
              borderRadius: BorderRadius.circular(Radii.lg),
            ),
      side: side,
      label: fillWidth
          ? SizedBox(width: double.infinity, child: centeredLabel)
          : centeredLabel,
      labelPadding: EdgeInsets.zero,
      onSelected: (_) => onSelected(item.id),
      padding: EdgeInsets.symmetric(
        horizontal: fillWidth ? Grid.quarter : Grid.twelve,
        vertical: chipVerticalPadding,
      ),
      visualDensity: minControlHeight > 0
          ? VisualDensity.standard
          : visualDensity,
      materialTapTargetSize: MaterialTapTargetSize.shrinkWrap,
    );

    final sizedChip = minControlHeight > 0
        ? ConstrainedBox(
            constraints: BoxConstraints(minHeight: minControlHeight),
            child: chip,
          )
        : chip;
    final control = fillWidth
        ? SizedBox(width: double.infinity, child: sizedChip)
        : sizedChip;
    if (!item.hasUnread) return control;
    return Semantics(
      label: '$text, unread',
      button: true,
      selected: isSelected,
      onTap: () => onSelected(item.id),
      excludeSemantics: true,
      child: Badge(child: control),
    );
  }
}
