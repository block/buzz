import 'package:flutter/material.dart';

import '../theme/theme.dart';

/// The horizontal padding list rows use, allowing grouped containers to replace
/// the page-level inset without changing each row.
class AppListInset extends InheritedWidget {
  const AppListInset({
    super.key,
    required this.horizontal,
    this.iconColumnWidth,
    this.sheetActions = false,
    required super.child,
  });

  final bool sheetActions;
  final double horizontal;

  static TextStyle? titleStyleOf(BuildContext context) =>
      context
              .dependOnInheritedWidgetOfExactType<AppListInset>()
              ?.sheetActions ==
          true
      ? sheetActionTextStyle.copyWith(color: context.colors.onSurface)
      : context.textTheme.bodyLarge;
  final double? iconColumnWidth;

  static double? iconWidthOf(BuildContext context) => context
      .dependOnInheritedWidgetOfExactType<AppListInset>()
      ?.iconColumnWidth;

  static double of(BuildContext context) =>
      context.dependOnInheritedWidgetOfExactType<AppListInset>()?.horizontal ??
      Grid.gutter;

  @override
  bool updateShouldNotify(AppListInset oldWidget) =>
      oldWidget.sheetActions != sheetActions ||
      oldWidget.horizontal != horizontal ||
      oldWidget.iconColumnWidth != iconColumnWidth;
}

/// Centers an action glyph in the same column as a channel settings row.
class AppListRowIcon extends StatelessWidget {
  const AppListRowIcon(this.icon, {super.key, this.color});

  final IconData icon;
  final Color? color;

  @override
  Widget build(BuildContext context) => SizedBox(
    width: AppListInset.iconWidthOf(context),
    child: Icon(icon, size: 22, color: color),
  );
}
