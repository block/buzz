/// How the tablet sidebar is shown beside a conversation.
enum ConversationSidebarMode {
  /// Tiled beside the conversation.
  docked,

  /// Out of view; the header toggle reveals it.
  hidden,

  /// Temporarily drawn over the conversation above a dimming scrim.
  overlay,
}

/// Where an open thread sits relative to its conversation.
enum ConversationThreadMode {
  /// No thread is open.
  none,

  /// Tiled to the right of the conversation.
  beside,

  /// Drawn over the conversation column.
  covering,
}

/// Narrowest conversation that keeps the sidebar docked beside it. Windows at
/// least ~850 pt wide dock it; an 11-inch iPad in portrait (834 pt) hides it.
const double conversationDockedMinWidth = 560;

/// Narrowest conversation that keeps a thread pane beside it.
const double conversationBesideThreadMinWidth = 480;

/// Default width of a thread pane shown beside its conversation.
double conversationThreadPaneWidth(double contentWidth) =>
    (contentWidth * 0.4).clamp(360.0, 440.0);

/// Narrowest thread pane the user can drag to.
const double conversationThreadPaneMinWidth = 320;

/// Resolved tablet pane geometry, in pane coordinates. Dividers are 1 pt.
class ConversationLayout {
  const ConversationLayout({
    required this.sidebar,
    required this.thread,
    required this.detailLeft,
    required this.detailWidth,
    required this.threadLeft,
    required this.threadWidth,
    required this.threadMaxWidth,
    required this.canDock,
  });

  /// How the sidebar is shown.
  final ConversationSidebarMode sidebar;

  /// Where the open thread sits, if any.
  final ConversationThreadMode thread;

  /// Left edge of the conversation column.
  final double detailLeft;

  /// Width of the conversation column.
  final double detailWidth;

  /// Left edge of the thread pane.
  final double threadLeft;

  /// Width of the thread pane.
  final double threadWidth;

  /// Widest a beside thread can be dragged while the sidebar keeps its mode
  /// and the conversation keeps [conversationBesideThreadMinWidth].
  final double threadMaxWidth;

  /// Whether there is room to dock the sidebar if the user wants it.
  final bool canDock;

  /// Resolves an iPad-style list/detail/thread layout.
  ///
  /// The sidebar docks while nothing is selected, or while the conversation
  /// keeps [conversationDockedMinWidth] (or [conversationBesideThreadMinWidth]
  /// beside a thread) and the user has not hidden it. Otherwise it hides, and
  /// [overlayOpen] draws it over the conversation. A thread sits beside the
  /// conversation when both fit, and covers the conversation column otherwise.
  ///
  /// [threadWidth] is the width the user dragged the thread pane to, if any;
  /// it is clamped so the conversation keeps its minimum width.
  ///
  /// [dockedMinWidth] replaces [conversationDockedMinWidth] for windows that
  /// keep their sidebar docked beside a narrower conversation, such as an
  /// iPhone Duo lying open in landscape.
  ///
  /// [holdDocked] is set only while the thread divider is being dragged, to
  /// whether the sidebar was docked when the drag began: true keeps it docked
  /// (and caps the thread so it still fits), false keeps it hidden. Dragging
  /// never makes the sidebar jump in or out; it settles once the drag ends.
  static ConversationLayout resolve({
    required double width,
    required double listWidth,
    required bool selected,
    required bool threadOpen,
    required bool userHidden,
    required bool overlayOpen,
    double? threadWidth,
    bool? holdDocked,
    double dockedMinWidth = conversationDockedMinWidth,
  }) {
    final soloMax = width - 1 - conversationBesideThreadMinWidth;
    final dockedMax = soloMax - listWidth - 1;
    var paneWidth = (threadWidth ?? conversationThreadPaneWidth(width)).clamp(
      conversationThreadPaneMinWidth,
      soloMax < conversationThreadPaneMinWidth
          ? conversationThreadPaneMinWidth
          : soloMax,
    );
    // Whether a thread fits beside is fixed by the default width, so a drag
    // never moves a thread between beside and covering.
    final threadBeside =
        selected &&
        threadOpen &&
        width - conversationThreadPaneWidth(width) - 1 >=
            conversationBesideThreadMinWidth;
    final holdingDock =
        threadBeside &&
        holdDocked == true &&
        dockedMax >= conversationThreadPaneMinWidth;
    if (holdingDock && paneWidth > dockedMax) paneWidth = dockedMax;
    final reservedForThread = threadBeside ? paneWidth + 1 : 0.0;
    final conversationWithSidebar = width - listWidth - 1 - reservedForThread;
    final canDock =
        conversationWithSidebar >=
        (threadBeside ? conversationBesideThreadMinWidth : dockedMinWidth);
    final docked =
        !selected ||
        (threadBeside && holdDocked != null
            ? holdingDock
            : canDock && !userHidden);
    final sidebar = docked
        ? ConversationSidebarMode.docked
        : overlayOpen
        ? ConversationSidebarMode.overlay
        : ConversationSidebarMode.hidden;
    final detailLeft = docked ? listWidth + 1 : 0.0;
    final detailWidth = width - detailLeft - reservedForThread;
    final thread = !selected || !threadOpen
        ? ConversationThreadMode.none
        : threadBeside
        ? ConversationThreadMode.beside
        : ConversationThreadMode.covering;
    return ConversationLayout(
      sidebar: sidebar,
      thread: thread,
      detailLeft: detailLeft,
      detailWidth: detailWidth,
      threadLeft: threadBeside ? width - paneWidth : detailLeft,
      threadWidth: threadBeside ? paneWidth : width - detailLeft,
      threadMaxWidth: docked && threadBeside ? dockedMax : soloMax,
      canDock: canDock,
    );
  }
}
