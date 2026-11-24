import 'package:flutter/material.dart';

import '../theme/theme.dart';

/// The horizontal padding list rows use, allowing grouped containers to replace
/// the page-level inset without changing each row.
class AppListInset extends InheritedWidget {
  const AppListInset({
    super.key,
    required this.horizontal,
    this.iconColumnWidth,
    required super.child,
  });

  final double horizontal;
  final double? iconColumnWidth;

  static double? iconWidthOf(BuildContext context) => context
      .dependOnInheritedWidgetOfExactType<AppListInset>()
      ?.iconColumnWidth;

  static double of(BuildContext context) =>
      context.dependOnInheritedWidgetOfExactType<AppListInset>()?.horizontal ??
      Grid.gutter;

  @override
  bool updateShouldNotify(AppListInset oldWidget) =>
      oldWidget.horizontal != horizontal ||
      oldWidget.iconColumnWidth != iconColumnWidth;
}
