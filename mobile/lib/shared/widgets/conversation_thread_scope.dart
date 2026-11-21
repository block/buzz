import 'package:flutter/widgets.dart';

/// Marks routes inside a thread pane, whose nested threads push in place.
class ConversationThreadScope extends InheritedWidget {
  const ConversationThreadScope({required super.child, super.key});

  /// Whether [context] is inside a thread pane.
  static bool isInside(BuildContext context) =>
      context.getInheritedWidgetOfExactType<ConversationThreadScope>() != null;

  @override
  bool updateShouldNotify(ConversationThreadScope oldWidget) => false;
}

/// Marks a full-screen page that shows over the conversation without leaving
/// it, such as the media viewer. Pushing one keeps the thread pane open.
mixin KeepsConversationThread<T> on PageRoute<T> {}
