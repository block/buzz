import 'package:flutter/material.dart';

/// Shares the scroll elevation state from [FrostedScaffold] with its header.
/// Keeping this separate lets all of the existing page bodies continue to own
/// their scroll views.
class FrostedHeaderScrollState extends InheritedNotifier<ValueNotifier<bool>> {
  const FrostedHeaderScrollState({
    required super.notifier,
    required super.child,
    super.key,
  });

  static bool isScrolledOf(BuildContext context) {
    return context
            .dependOnInheritedWidgetOfExactType<FrostedHeaderScrollState>()
            ?.notifier
            ?.value ??
        false;
  }
}
