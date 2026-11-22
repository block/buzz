import 'package:flutter/widgets.dart';

/// Selection belongs to the workspace, rather than to a particular display pose.
class ConversationPaneController extends ChangeNotifier {
  /// Identifies the selected conversation's native header across layouts.
  final nativeHeaderKey = GlobalKey();
  String? selectedId;
  WidgetBuilder? detailBuilder;
  bool animateSelection = true;
  int _request = 0;
  // Deep-link requests may target the same item after its nested route popped.
  // A fresh page identity reruns initialization without clearing the selection
  // when Navigator removes the previous page.
  LocalKey get selectionKey => ValueKey((selectedId, _request));

  /// The thread a tablet pane shows beside, or over, the selected conversation.
  WidgetBuilder? threadBuilder;
  int _threadRequest = 0;

  /// A fresh page identity for each opened thread.
  LocalKey get threadKey => ValueKey(('thread', _threadRequest));

  /// Whether a hidden tablet sidebar is drawn over the conversation.
  bool sidebarOverlay = false;

  /// Whether the tablet thread pane fills the window, over the list and the
  /// conversation. Each newly opened thread starts beside its conversation.
  bool threadExpanded = false;

  void open(
    String id,
    WidgetBuilder builder, {
    bool animate = true,
    bool reopen = false,
  }) {
    if (id == selectedId && !reopen) {
      hideSidebarOverlay();
      return;
    }
    if (reopen) _request++;
    selectedId = id;
    detailBuilder = builder;
    animateSelection = animate;
    threadBuilder = null;
    threadExpanded = false;
    sidebarOverlay = false;
    notifyListeners();
  }

  void clear() {
    if (selectedId == null) return;
    selectedId = null;
    detailBuilder = null;
    threadBuilder = null;
    threadExpanded = false;
    sidebarOverlay = false;
    notifyListeners();
  }

  /// Shows [builder] in the thread pane, replacing any thread already there.
  void openThread(WidgetBuilder builder) {
    _threadRequest++;
    threadBuilder = builder;
    threadExpanded = false;
    sidebarOverlay = false;
    notifyListeners();
  }

  /// Closes the thread pane.
  void closeThread() {
    if (threadBuilder == null) return;
    threadBuilder = null;
    threadExpanded = false;
    notifyListeners();
  }

  /// Lets the thread pane fill the window, or returns it beside the
  /// conversation.
  void setThreadExpanded(bool expanded) {
    if (threadExpanded == expanded || (expanded && threadBuilder == null)) {
      return;
    }
    threadExpanded = expanded;
    notifyListeners();
  }

  /// Draws a hidden tablet sidebar over the selected conversation.
  void showSidebarOverlay() {
    if (sidebarOverlay || selectedId == null) return;
    sidebarOverlay = true;
    notifyListeners();
  }

  /// Dismisses a sidebar drawn over the conversation.
  void hideSidebarOverlay() {
    if (!sidebarOverlay) return;
    sidebarOverlay = false;
    notifyListeners();
  }
}
