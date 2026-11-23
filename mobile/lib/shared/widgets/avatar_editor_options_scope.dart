import 'package:flutter/widgets.dart';

/// Enables side-rail options for the full-screen photo editor.
class AvatarEditorOptionsScope extends InheritedWidget {
  const AvatarEditorOptionsScope({
    required this.interactive,
    required super.child,
    super.key,
  });

  final bool interactive;

  static AvatarEditorOptionsScope? maybeOf(BuildContext context) =>
      context.dependOnInheritedWidgetOfExactType<AvatarEditorOptionsScope>();

  @override
  bool updateShouldNotify(AvatarEditorOptionsScope oldWidget) =>
      interactive != oldWidget.interactive;
}
