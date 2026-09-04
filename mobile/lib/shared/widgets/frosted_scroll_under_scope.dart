import 'package:flutter/widgets.dart';

/// Supplies whether vertical content has moved beneath a frosted app bar.
class FrostedScrollUnderScope extends InheritedWidget {
  const FrostedScrollUnderScope({
    super.key,
    required this.isScrolledUnder,
    required super.child,
  });

  final bool isScrolledUnder;

  static FrostedScrollUnderScope? maybeOf(BuildContext context) =>
      context.dependOnInheritedWidgetOfExactType<FrostedScrollUnderScope>();

  @override
  bool updateShouldNotify(FrostedScrollUnderScope oldWidget) =>
      isScrolledUnder != oldWidget.isScrolledUnder;
}
