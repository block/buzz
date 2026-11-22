import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';

import '../theme/theme.dart';
import 'adaptive_conversation_pane.dart';
import 'app_route_observer.dart';
import 'conversation_pane_modals.dart';
import 'conversation_thread_scope.dart';

/// A tablet pane's thread, beside its conversation or over it.
///
/// The thread is the second page of its own navigator, so it keeps Back and
/// the edge swipe, and nested threads stack inside the pane. Popping the
/// thread closes the pane.
class ConversationThreadPane extends HookWidget {
  const ConversationThreadPane({
    required this.controller,
    required this.navigatorKey,
    required this.covering,
    required this.modalRoutes,
    super.key,
  });

  final ConversationPaneController controller;
  final GlobalKey<NavigatorState> navigatorKey;

  /// Whether the thread covers the conversation rather than sitting beside it.
  final bool covering;

  /// Tracks pop-up routes over the thread, which hide its neighbours.
  final PaneModalRoutes modalRoutes;

  @override
  Widget build(BuildContext context) {
    final routeObserver = useMemoized(appRouteObserver.createNestedObserver);
    final builder = controller.threadBuilder;
    // The thread's routes block screen readers from everything painted before
    // them in their container. Their own container keeps that block inside
    // the pane, so the conversation beside it stays reachable; the pane hides
    // its neighbours itself while a sheet is open over the thread.
    return ConversationThreadScope(
      child: Semantics(
        container: true,
        explicitChildNodes: true,
        child: Navigator(
          key: navigatorKey,
          observers: [routeObserver, modalRoutes],
          pages: [
            MaterialPage<void>(
              key: const ValueKey('conversation-thread-placeholder'),
              // Beside the conversation, the pane owns its own surface. Over
              // it, the conversation shows through while the thread moves.
              child: covering
                  ? const SizedBox.shrink()
                  : ColoredBox(color: context.colors.surface),
            ),
            if (builder != null)
              MaterialPage<void>(
                key: controller.threadKey,
                child: Builder(builder: builder),
              ),
          ],
          onDidRemovePage: (page) {
            if (page.key == controller.threadKey) controller.closeThread();
          },
        ),
      ),
    );
  }
}
