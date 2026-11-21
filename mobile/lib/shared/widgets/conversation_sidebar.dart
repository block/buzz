import 'package:flutter/material.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../theme/buzz_icons.dart';
import 'adaptive_conversation_pane.dart';
import 'conversation_layout.dart';
import 'ios_navigation_bar.dart';

/// Whether the user hid the tablet sidebar where it would otherwise dock.
/// Kept for the app session, across communities and rotations.
final conversationSidebarHiddenProvider =
    NotifierProvider<ConversationSidebarHidden, bool>(
      ConversationSidebarHidden.new,
    );

/// Holds whether the user hid the tablet sidebar.
class ConversationSidebarHidden extends Notifier<bool> {
  @override
  bool build() => false;

  // A setter would read as field assignment at call sites; keep it a method.
  // ignore: use_setters_to_change_properties
  void set(bool hidden) => state = hidden;
}

/// Exposes the tablet sidebar's state to the conversation column's header.
class ConversationSidebarScope extends InheritedWidget {
  const ConversationSidebarScope({
    required this.mode,
    required this.canDock,
    required this.controller,
    required this.hidden,
    required super.child,
    super.key,
  });

  /// How the sidebar is shown right now.
  final ConversationSidebarMode mode;

  /// Whether the sidebar would fit docked if the user showed it.
  final bool canDock;

  /// The pane that owns the sidebar overlay.
  final ConversationPaneController controller;

  /// The user's hide choice.
  final ConversationSidebarHidden hidden;

  /// The nearest scope, or null outside a tablet conversation column.
  static ConversationSidebarScope? maybeOf(BuildContext context) =>
      context.dependOnInheritedWidgetOfExactType<ConversationSidebarScope>();

  /// Hides or shows the sidebar; see [toggleFor].
  void toggle() => toggleFor(
    mode: mode,
    canDock: canDock,
    controller: controller,
    hidden: hidden,
  );

  /// Hides a docked sidebar, docks a hidden one where it fits, and otherwise
  /// reveals or dismisses it over the conversation.
  static void toggleFor({
    required ConversationSidebarMode mode,
    required bool canDock,
    required ConversationPaneController controller,
    required ConversationSidebarHidden hidden,
  }) {
    switch (mode) {
      case ConversationSidebarMode.docked:
        hidden.set(true);
      case ConversationSidebarMode.hidden:
        if (canDock) {
          hidden.set(false);
        } else {
          controller.showSidebarOverlay();
        }
      case ConversationSidebarMode.overlay:
        controller.hideSidebarOverlay();
    }
  }

  /// The toggle's tooltip and screen-reader label.
  String get toggleLabel =>
      mode == ConversationSidebarMode.hidden ? 'Show sidebar' : 'Hide sidebar';

  @override
  bool updateShouldNotify(ConversationSidebarScope oldWidget) =>
      mode != oldWidget.mode ||
      canDock != oldWidget.canDock ||
      controller != oldWidget.controller ||
      hidden != oldWidget.hidden;
}

/// The sidebar toggle for a header on its tablet pane's selected page, or
/// null elsewhere (pushed pages keep Back, and phones have no sidebar).
ConversationSidebarScope? conversationSidebarToggleOf(BuildContext context) {
  final scope = ConversationSidebarScope.maybeOf(context);
  if (scope == null || !ConversationPaneScope.isBaseRouteOf(context)) {
    return null;
  }
  return scope;
}

/// Native header action for [scope]'s toggle.
IosNavigationAction conversationSidebarAction(ConversationSidebarScope scope) =>
    IosNavigationAction(
      label: scope.toggleLabel,
      icon: BuzzIcons.layoutSidebar,
      onPressed: scope.toggle,
    );

/// Material header button for [scope]'s toggle.
Widget conversationSidebarButton(ConversationSidebarScope scope) => IconButton(
  key: const ValueKey('conversation-sidebar-toggle'),
  tooltip: scope.toggleLabel,
  icon: const Icon(BuzzIcons.layoutSidebar),
  onPressed: scope.toggle,
);
