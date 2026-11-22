import 'package:flutter/material.dart';

import '../theme/buzz_icons.dart';
import 'ios_navigation_bar.dart';

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

/// Lets the pages of a tablet thread pane fill the window with the thread, or
/// return it beside the conversation. Absent where the thread already covers
/// the conversation for lack of room, and outside thread panes.
class ConversationThreadExpansion extends InheritedWidget {
  const ConversationThreadExpansion({
    required this.expanded,
    required this.onToggle,
    required super.child,
    super.key,
  });

  /// Whether the thread fills the window.
  final bool expanded;

  /// Expands or collapses the thread.
  final VoidCallback onToggle;

  /// Offers [child]'s pages the toggle where the thread can fill the window
  /// or already does.
  static Widget offer({
    required bool available,
    required bool expanded,
    required VoidCallback onToggle,
    required Widget child,
  }) => available
      ? ConversationThreadExpansion(
          expanded: expanded,
          onToggle: onToggle,
          child: child,
        )
      : child;

  /// The nearest thread pane that can expand, or null.
  static ConversationThreadExpansion? maybeOf(BuildContext context) =>
      context.dependOnInheritedWidgetOfExactType<ConversationThreadExpansion>();

  /// The toggle's tooltip and screen-reader label.
  String get label => expanded ? 'Show beside conversation' : 'Expand thread';

  IconData get _icon =>
      expanded ? BuzzIcons.arrowsDiagonalMinimize : BuzzIcons.arrowsDiagonal;

  /// Native header action for the toggle.
  IosNavigationAction get action =>
      IosNavigationAction(label: label, icon: _icon, onPressed: onToggle);

  /// Material header button for the toggle.
  Widget get button => IconButton(
    key: const ValueKey('conversation-thread-expand'),
    tooltip: label,
    icon: Icon(_icon),
    onPressed: onToggle,
  );

  @override
  bool updateShouldNotify(ConversationThreadExpansion oldWidget) =>
      expanded != oldWidget.expanded || onToggle != oldWidget.onToggle;
}
