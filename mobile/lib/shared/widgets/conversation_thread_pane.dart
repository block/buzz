import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';

import '../theme/theme.dart';
import 'adaptive_conversation_pane.dart';
import 'app_route_observer.dart';
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
    super.key,
  });

  final ConversationPaneController controller;
  final GlobalKey<NavigatorState> navigatorKey;

  /// Whether the thread covers the conversation rather than sitting beside it.
  final bool covering;

  @override
  Widget build(BuildContext context) {
    final routeObserver = useMemoized(appRouteObserver.createNestedObserver);
    final builder = controller.threadBuilder;
    return ConversationThreadScope(
      child: Navigator(
        key: navigatorKey,
        observers: [routeObserver],
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
    );
  }
}
