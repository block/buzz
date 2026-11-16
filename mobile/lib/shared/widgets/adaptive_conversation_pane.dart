import 'package:flutter/material.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:duo_kit/duo_kit.dart';

import 'duo_toolbar.dart';
import 'app_route_observer.dart';
import 'conversation_layout.dart';
import 'conversation_sidebar.dart';
import 'conversation_thread_divider.dart';
import 'conversation_thread_pane.dart';
import 'conversation_thread_scope.dart';
import 'covered_route_keyboard_insets.dart';
import 'ios_navigation_metrics.dart';
import 'duo_workspace_scaffold.dart';

import '../theme/theme.dart';

/// Duo uses the phone navigation model in portrait, even on its large display.
/// Callers inside a pane supply the full window size.
bool usesConversationPanes(
  BuildContext context, {
  FoldStrip? strip,
  Size? windowSize,
}) {
  final size = windowSize ?? MediaQuery.sizeOf(context);
  final rail = strip ?? FoldGeometry.of(context).strip;
  // The portrait Duo status cluster becomes a tall top inset, not a side
  // strip. Keep that phone layout compact without excluding ordinary iPads.
  final phoneStatusCluster = MediaQuery.viewPaddingOf(context).top > 50;
  if (defaultTargetPlatform == TargetPlatform.iOS &&
      size.height >= size.width &&
      (rail != null || phoneStatusCluster || size.shortestSide < 600)) {
    return false;
  }
  final bookDivision = bookDivisionOf(context);
  // A landscape phone is wide enough for the breakpoint, but not for two
  // usable panes. Fold geometry keeps Duo eligible in its supported poses.
  if (size.shortestSide < 600 && rail == null && bookDivision == null) {
    return false;
  }
  return bookDivision != null || size.width - (rail?.width ?? 0) >= 640;
}

/// The active vertical fold, in window coordinates, when held like a book.
Rect? bookDivisionOf(BuildContext context) {
  final geometry = FoldGeometry.of(context);
  final division = geometry.division;
  if (division != null &&
      division.axis == Axis.vertical &&
      division.isHalfOpened) {
    return division.bounds;
  }
  final native = IosNavigationMetrics.of(context);
  return native.windowSize == geometry.size ? native.bookDivision : null;
}

final _compactPaneNavigators = Expando<GlobalKey<NavigatorState>>();

/// Unwinds a compact destination for a shortcut, without bypassing page vetoes.
/// Returns false for other layouts or when a nested page refuses to close.
bool popCompactConversationDestination(Route<dynamic> route) {
  final navigator = _compactPaneNavigators[route]?.currentState;
  if (navigator == null) return false;
  var cleared = false;
  navigator.popUntil((nested) {
    if (nested.popDisposition == RoutePopDisposition.doNotPop) return true;
    cleared = nested.isFirst;
    return cleared;
  });
  return cleared;
}

/// Selection belongs to the workspace, rather than to a particular display pose.
class ConversationPaneController extends ChangeNotifier {
  final _nativeHeaderKey = GlobalKey();
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
    sidebarOverlay = false;
    notifyListeners();
  }

  void clear() {
    if (selectedId == null) return;
    selectedId = null;
    detailBuilder = null;
    threadBuilder = null;
    sidebarOverlay = false;
    notifyListeners();
  }

  /// Shows [builder] in the thread pane, replacing any thread already there.
  void openThread(WidgetBuilder builder) {
    _threadRequest++;
    threadBuilder = builder;
    sidebarOverlay = false;
    notifyListeners();
  }

  /// Closes the thread pane.
  void closeThread() {
    if (threadBuilder == null) return;
    threadBuilder = null;
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

class ConversationPaneScope
    extends InheritedNotifier<ConversationPaneController> {
  ConversationPaneScope({
    required ConversationPaneController controller,
    required Widget child,
    bool active = true,
    super.key,
  }) : super(
         notifier: controller,
         child: _ConversationPaneActive(active: active, child: child),
       );

  /// Only the selected page owns the pane's implicit root navigation.
  /// Imperatively pushed pages retain ordinary Back navigation.
  static bool isBaseRouteOf(BuildContext context) {
    final controller = maybeOf(context);
    final settings = ModalRoute.of(context)?.settings;
    return controller?.selectedId != null &&
        settings is Page &&
        settings.key == controller!.selectionKey;
  }

  /// Keeps the native header surface alive across immediate sidebar selections.
  /// Conversation state still belongs to its separately keyed route.
  static GlobalKey? nativeHeaderKeyOf(BuildContext context) {
    final controller = maybeOf(context);
    if (!isBaseRouteOf(context) || !_isWide(context)) return null;
    return controller?._nativeHeaderKey;
  }

  /// A replaced zero-duration page can remain mounted until frame cleanup.
  /// It must relinquish the shared header before the next page adopts it.
  static bool isReplacedBaseRouteOf(BuildContext context) {
    final settings = ModalRoute.of(context)?.settings;
    return settings is _ConversationDetailPage &&
        !isBaseRouteOf(context) &&
        _isWide(context);
  }

  static bool _isWide(BuildContext context) => usesConversationPanes(
    context,
    windowSize:
        View.of(context).physicalSize / View.of(context).devicePixelRatio,
  );

  /// Depends only on [active], which flips whenever a route covers the pane,
  /// so readers of [maybeOf] (rows, the list) don't rebuild with it.
  static bool isActiveOf(BuildContext context) =>
      context
          .dependOnInheritedWidgetOfExactType<_ConversationPaneActive>()
          ?.active ??
      true;

  static ConversationPaneController? maybeOf(BuildContext context) => context
      .dependOnInheritedWidgetOfExactType<ConversationPaneScope>()
      ?.notifier;
}

class _ConversationPaneActive extends InheritedWidget {
  const _ConversationPaneActive({required this.active, required super.child});

  final bool active;

  @override
  bool updateShouldNotify(_ConversationPaneActive oldWidget) =>
      active != oldWidget.active;
}

/// Keeps both the list and its detail navigator mounted across compact/regular
/// layouts, so opening or closing Duo does not recreate the composer or thread.
class AdaptiveConversationPane extends HookConsumerWidget {
  const AdaptiveConversationPane({
    required this.controller,
    required this.list,
    this.active = true,
    this.strip,
    this.emptyLabel = 'Select a conversation',
    this.emptyContent,
    this.listWidthFraction,
    this.preferredListWidth,
    this.persistentSelection = false,
    this.listToolbarSlots = 0,
    super.key,
  });

  final bool persistentSelection;

  /// Rail slots reserved for workspace controls that stay active in wide mode.
  final int listToolbarSlots;

  /// Optional list share of the usable width; defaults to the compact sidebar.
  final double? listWidthFraction;

  /// Preferred sidebar width, capped at half the usable area for narrow windows.
  final double? preferredListWidth;
  final String emptyLabel;
  final Widget? emptyContent;
  final FoldStrip? strip;
  final ConversationPaneController controller;
  final Widget list;
  final bool active;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final routeObserver = useMemoized(appRouteObserver.createNestedObserver);
    final navigator = useMemoized(() => GlobalKey<NavigatorState>());
    final ownerRoute = ModalRoute.of(context);
    useEffect(() {
      return () {
        if (ownerRoute != null &&
            _compactPaneNavigators[ownerRoute] == navigator) {
          _compactPaneNavigators[ownerRoute] = null;
        }
      };
    }, [ownerRoute, navigator]);
    // The compact detail route that fully covers the list. While a detail
    // pushes, pops or follows a back swipe, the list stays painted beneath it.
    final coveringDetail = useValueNotifier<Route<Object?>?>(null);
    // Compact detail routes still mounted. A pop clears the selection as it
    // starts, so the detail stays onstage until its reverse transition ends.
    // Not disposed with the pane: a detail unmounting with the pane reports
    // after the frame, once the pane is gone.
    final mountedDetails = useMemoized(() => ValueNotifier(0));
    final threadNavigator = useMemoized(() => GlobalKey<NavigatorState>());
    final sidebarHidden = ref.watch(conversationSidebarHiddenProvider);
    final threadWidth = ref.watch(conversationThreadWidthProvider);
    // While the thread divider is dragged: whether the sidebar was docked.
    final resizeHold = useState<bool?>(null);
    // The overlay is modal: focus moves into the sidebar while it is open,
    // so Escape reaches the pane, and returns to where it was afterwards.
    final listFocus = useFocusNode(debugLabel: 'conversation-list');
    useEffect(() {
      var open = controller.sidebarOverlay;
      FocusNode? previous;
      void sync() {
        if (controller.sidebarOverlay == open) return;
        open = controller.sidebarOverlay;
        if (open) {
          previous = FocusManager.instance.primaryFocus;
        }
        final target = open ? listFocus : previous;
        if (!open) previous = null;
        WidgetsBinding.instance.addPostFrameCallback((_) {
          if (target != null && target.context != null) target.requestFocus();
        });
      }

      controller.addListener(sync);
      return () => controller.removeListener(sync);
    }, [controller]);
    return ConversationPaneScope(
      controller: controller,
      active: active && (ModalRoute.isCurrentOf(context) ?? true),
      child: ListenableBuilder(
        listenable: controller,
        builder: (context, _) => LayoutBuilder(
          builder: (context, constraints) {
            final stripWidth = strip?.width ?? 0;
            final leftInset = strip?.side == FoldStripSide.left
                ? stripWidth
                : 0.0;
            final rightInset = strip?.side == FoldStripSide.right
                ? stripWidth
                : 0.0;
            final contentWidth = constraints.maxWidth - stripWidth;
            final threadBuilder = controller.threadBuilder;
            final wide = usesConversationPanes(context, strip: strip);
            final book = wide ? bookDivisionOf(context) : null;
            final selected = controller.selectedId;
            final threadOpen = selected != null && threadBuilder != null;
            if (ownerRoute != null) {
              _compactPaneNavigators[ownerRoute] =
                  active && !wide && selected != null ? navigator : null;
            }
            // Pane surfaces meet at the fold so their header backdrops remain
            // continuous. Content insets below keep controls out of its bounds.
            // The default sidebar fits Home's 256 pt navigation bar inside
            // the standard Grid.xs window margins.
            final listWidth = book != null
                ? book.center.dx - leftInset
                : wide
                ? listWidthFraction == null
                      ? preferredListWidth?.clamp(
                              0.0,
                              (contentWidth - 1) / 2,
                            ) ??
                            (contentWidth / 3).clamp(256.0 + 2 * Grid.xs, 360.0)
                      : (contentWidth - 1) * listWidthFraction!
                : constraints.maxWidth;
            // An unfolded tablet window can hide its sidebar and show a thread
            // beside the conversation. Duo keeps its hinge-bound panes, and a
            // persistent selection (Settings) keeps its list as navigation.
            final tablet =
                wide && book == null && strip == null && !persistentSelection;
            final layout = tablet
                ? ConversationLayout.resolve(
                    width: constraints.maxWidth,
                    listWidth: listWidth,
                    selected: selected != null,
                    threadOpen: threadOpen,
                    userHidden: sidebarHidden,
                    overlayOpen: controller.sidebarOverlay,
                    threadWidth: threadWidth,
                    holdDocked: resizeHold.value,
                  )
                : null;
            final docked =
                layout == null ||
                layout.sidebar == ConversationSidebarMode.docked;
            // Once the sidebar docks, for example after rotating, a pending
            // overlay request is spent; otherwise it would reappear later.
            if (layout != null && docked && controller.sidebarOverlay) {
              WidgetsBinding.instance.addPostFrameCallback(
                (_) => controller.hideSidebarOverlay(),
              );
            }
            final detailLeft =
                layout?.detailLeft ??
                book?.center.dx ??
                (wide ? listWidth + leftInset + 1 : 0.0);
            final detailWidth =
                layout?.detailWidth ?? constraints.maxWidth - detailLeft;
            void toggleSidebar() => ConversationSidebarScope.toggleFor(
              mode: layout!.sidebar,
              canDock: layout.canDock,
              controller: controller,
              hidden: ref.read(conversationSidebarHiddenProvider.notifier),
            );
            final pane = TweenAnimationBuilder<_PaneGeometry>(
              tween: _PaneGeometryTween(
                end: _PaneGeometry(
                  listLeft:
                      docked ||
                          layout.sidebar == ConversationSidebarMode.overlay
                      ? 0
                      : -listWidth - 1,
                  listWidth: listWidth,
                  detailLeft: detailLeft,
                  detailWidth: detailWidth,
                  threadLeft: layout?.threadLeft ?? detailLeft,
                  threadWidth: layout?.threadWidth ?? detailWidth,
                ),
              ),
              duration:
                  wide &&
                      resizeHold.value == null &&
                      !MediaQuery.disableAnimationsOf(context)
                  ? const Duration(milliseconds: 240)
                  : Duration.zero,
              curve: Curves.easeInOutCubic,
              builder: (context, geometry, _) {
                final listWidth = geometry.listWidth;
                final detailLeft = geometry.detailLeft;
                final detailWidth = geometry.detailWidth;
                // A hidden sidebar leaves input, focus and the semantics tree
                // as soon as it starts to slide away, and stops painting once
                // it is out of view.
                final sidebarAway =
                    layout != null &&
                    layout.sidebar == ConversationSidebarMode.hidden;
                final sidebarGone =
                    sidebarAway && geometry.listLeft <= -listWidth;
                final overlaid =
                    layout != null &&
                    !docked &&
                    (layout.sidebar == ConversationSidebarMode.overlay ||
                        geometry.listLeft > -listWidth) &&
                    detailLeft <= 0.5;
                final threadCovers =
                    layout?.thread == ConversationThreadMode.covering ||
                    (layout == null && threadOpen);
                final listPane = Positioned(
                  key: const ValueKey('conversation-list'),
                  left: geometry.listLeft,
                  top: 0,
                  bottom: 0,
                  width: wide ? listWidth + leftInset : listWidth,
                  child: Focus(
                    focusNode: listFocus,
                    skipTraversal: true,
                    child: Offstage(
                      offstage: sidebarGone,
                      child: DecoratedBox(
                        decoration: BoxDecoration(
                          boxShadow: overlaid
                              ? [
                                  BoxShadow(
                                    blurRadius: Grid.xs + Grid.xxs,
                                    color: context.colors.shadow.withValues(
                                      alpha: 0.25,
                                    ),
                                  ),
                                ]
                              : null,
                        ),
                        child: ValueListenableBuilder(
                          valueListenable: coveringDetail,
                          builder: (context, covering, list) => Offstage(
                            offstage:
                                !wide &&
                                selected != null &&
                                (covering?.isActive ?? false),
                            child: list,
                          ),
                          child: _CoveredList(
                            covered: (!wide && selected != null) || sidebarAway,
                            child: MediaQueryOverride(
                              transform: (media) => media.copyWith(
                                size: Size(listWidth, constraints.maxHeight),
                              ),
                              child: DuoToolbarScope(
                                strip: strip,
                                contentInsets: EdgeInsets.only(
                                  left: leftInset,
                                  right: wide
                                      ? (book?.width ?? 0) / 2
                                      : rightInset,
                                ),
                                active:
                                    active &&
                                    (selected == null ||
                                        (wide &&
                                            (persistentSelection ||
                                                listToolbarSlots > 0))) &&
                                    (ModalRoute.isCurrentOf(context) ?? true),
                                child: list,
                              ),
                            ),
                          ),
                        ),
                      ),
                    ),
                  ),
                );
                Widget detail = Navigator(
                  requestFocus: selected != null,
                  key: navigator,
                  observers: [routeObserver],
                  pages: [
                    if (!(wide && persistentSelection && selected != null))
                      MaterialPage<void>(
                        key: const ValueKey('conversation-placeholder'),
                        // Compact panes show the list through this page while
                        // a detail pushes or pops.
                        child: !wide
                            ? const SizedBox.shrink()
                            : Material(
                                color: context.colors.surface,
                                child: Center(
                                  child:
                                      emptyContent ??
                                      Text(
                                        emptyLabel,
                                        textAlign: TextAlign.center,
                                        style: Theme.of(context)
                                            .textTheme
                                            .bodyLarge
                                            ?.copyWith(
                                              color: Theme.of(
                                                context,
                                              ).colorScheme.onSurfaceVariant,
                                              decoration: TextDecoration.none,
                                            ),
                                      ),
                                ),
                              ),
                      ),
                    if (selected != null)
                      _ConversationDetailPage(
                        key: controller.selectionKey,
                        animate: !wide && controller.animateSelection,
                        child: _CoveringDetail(
                          covering: coveringDetail,
                          mounted: mountedDetails,
                          builder: controller.detailBuilder!,
                        ),
                      ),
                  ],
                  onDidRemovePage: (page) {
                    if (page.key == controller.selectionKey) {
                      controller.clear();
                    }
                  },
                );
                if (layout != null) {
                  detail = ConversationSidebarScope(
                    mode: layout.sidebar,
                    canDock: layout.canDock,
                    controller: controller,
                    hidden: ref.read(
                      conversationSidebarHiddenProvider.notifier,
                    ),
                    child: detail,
                  );
                }
                final detailPane = Positioned(
                  key: const ValueKey('conversation-detail'),
                  left: detailLeft,
                  top: 0,
                  bottom: 0,
                  width: detailWidth,
                  child: ValueListenableBuilder<int>(
                    valueListenable: mountedDetails,
                    builder: (context, mounted, child) {
                      final shown = wide || selected != null || mounted > 0;
                      return Offstage(
                        offstage: !shown,
                        child: TickerMode(enabled: shown, child: child!),
                      );
                    },
                    child: MediaQueryOverride(
                      transform: (media) => media.copyWith(
                        size: Size(detailWidth, constraints.maxHeight),
                      ),
                      child: DuoToolbarScope(
                        strip: strip,
                        leadingSlots: wide ? listToolbarSlots : 0,
                        contentInsets: EdgeInsets.only(
                          left: wide ? (book?.width ?? 0) / 2 : leftInset,
                          right: rightInset,
                        ),
                        active:
                            active &&
                            selected != null &&
                            !(threadCovers) &&
                            (ModalRoute.isCurrentOf(context) ?? true),
                        child: Semantics(
                          container: true,
                          explicitChildNodes: true,
                          child: PopScope<Object?>(
                            canPop:
                                !active ||
                                selected == null ||
                                (wide && persistentSelection && !threadOpen),
                            onPopInvokedWithResult: (didPop, result) {
                              if (didPop || !active) return;
                              if (overlaid) {
                                controller.hideSidebarOverlay();
                              } else if (threadOpen) {
                                threadNavigator.currentState?.maybePop(result);
                              } else if (selected != null &&
                                  !(wide && persistentSelection)) {
                                navigator.currentState!.maybePop(result);
                              }
                            },
                            child: ExcludeFocus(
                              excluding:
                                  selected == null || threadCovers || overlaid,
                              child: ExcludeSemantics(
                                excluding: threadCovers || overlaid,
                                child: NavigatorPopHandler<Object?>(
                                  enabled: wide && persistentSelection,
                                  // A disabled handler is still told about
                                  // a blocked pop; the PopScope above owns
                                  // Back then (overlay, thread, selection).
                                  onPopWithResult: (result) {
                                    if (wide && persistentSelection) {
                                      navigator.currentState!.pop(result);
                                    }
                                  },
                                  child: detail,
                                ),
                              ),
                            ),
                          ),
                        ),
                      ),
                    ),
                  ),
                );
                return Stack(
                  fit: StackFit.expand,
                  children: [
                    // Keys keep each pane's state while the list moves above
                    // the conversation to draw over it.
                    if (!overlaid) listPane,
                    detailPane,
                    if (wide && docked)
                      Positioned(
                        left: detailLeft - 1,
                        top: 0,
                        bottom: 0,
                        width: 1,
                        child: IgnorePointer(
                          child: ColoredBox(
                            color: context.colors.outlineVariant,
                          ),
                        ),
                      ),
                    if (threadOpen) ...[
                      Positioned(
                        key: const ValueKey('conversation-thread'),
                        left: geometry.threadLeft,
                        top: 0,
                        bottom: 0,
                        width: geometry.threadWidth,
                        child: MediaQueryOverride(
                          transform: (media) => media.copyWith(
                            size: Size(
                              geometry.threadWidth,
                              constraints.maxHeight,
                            ),
                          ),
                          child: DuoToolbarScope(
                            strip: strip,
                            active:
                                active &&
                                (ModalRoute.isCurrentOf(context) ?? true),
                            child: ExcludeFocus(
                              excluding: overlaid,
                              child: ExcludeSemantics(
                                excluding: overlaid,
                                child: ConversationThreadPane(
                                  controller: controller,
                                  navigatorKey: threadNavigator,
                                  covering: threadCovers,
                                ),
                              ),
                            ),
                          ),
                        ),
                      ),
                      // Drawn above both panes so the handle straddles them.
                      if (!threadCovers)
                        Positioned(
                          key: const ValueKey('conversation-thread-divider'),
                          left:
                              geometry.threadLeft -
                              1 -
                              (conversationThreadHandleWidth - 1) / 2,
                          top: 0,
                          bottom: 0,
                          width: conversationThreadHandleWidth,
                          child: ConversationThreadDivider(
                            width: layout!.threadWidth,
                            maxWidth: layout.threadMaxWidth,
                            onDragStart: () => resizeHold.value =
                                layout.sidebar ==
                                ConversationSidebarMode.docked,
                            onResize: ref
                                .read(conversationThreadWidthProvider.notifier)
                                .set,
                            onDragEnd: () => resizeHold.value = null,
                          ),
                        ),
                    ],
                    if (overlaid)
                      Positioned.fill(
                        key: const ValueKey('conversation-sidebar-scrim'),
                        child: Semantics(
                          button: true,
                          label: 'Close sidebar',
                          onTap: controller.hideSidebarOverlay,
                          excludeSemantics: true,
                          child: GestureDetector(
                            behavior: HitTestBehavior.opaque,
                            onTap: controller.hideSidebarOverlay,
                            child: ColoredBox(
                              color: context.colors.scrim.withValues(
                                alpha:
                                    0.25 *
                                    ((geometry.listLeft + listWidth + 1) /
                                            (listWidth + 1))
                                        .clamp(0.0, 1.0),
                              ),
                            ),
                          ),
                        ),
                      ),
                    if (overlaid) listPane,
                  ],
                );
              },
            );
            // Always present, so gaining bindings never remounts the panes.
            return CallbackShortcuts(
              bindings: {
                if (layout != null && selected != null) ...{
                  const SingleActivator(
                    LogicalKeyboardKey.keyS,
                    control: true,
                    meta: true,
                  ): toggleSidebar,
                  if (layout.sidebar == ConversationSidebarMode.overlay)
                    const SingleActivator(LogicalKeyboardKey.escape):
                        controller.hideSidebarOverlay,
                },
              },
              child: pane,
            );
          },
        ),
      ),
    );
  }
}

/// Animated tablet pane positions, in pane coordinates.
@immutable
class _PaneGeometry {
  const _PaneGeometry({
    required this.listLeft,
    required this.listWidth,
    required this.detailLeft,
    required this.detailWidth,
    required this.threadLeft,
    required this.threadWidth,
  });

  final double listLeft;
  final double listWidth;
  final double detailLeft;
  final double detailWidth;
  final double threadLeft;
  final double threadWidth;

  static _PaneGeometry lerp(_PaneGeometry a, _PaneGeometry b, double t) {
    double mix(double x, double y) => x + (y - x) * t;
    return _PaneGeometry(
      listLeft: mix(a.listLeft, b.listLeft),
      listWidth: mix(a.listWidth, b.listWidth),
      detailLeft: mix(a.detailLeft, b.detailLeft),
      detailWidth: mix(a.detailWidth, b.detailWidth),
      threadLeft: mix(a.threadLeft, b.threadLeft),
      threadWidth: mix(a.threadWidth, b.threadWidth),
    );
  }

  @override
  bool operator ==(Object other) =>
      other is _PaneGeometry &&
      other.listLeft == listLeft &&
      other.listWidth == listWidth &&
      other.detailLeft == detailLeft &&
      other.detailWidth == detailWidth &&
      other.threadLeft == threadLeft &&
      other.threadWidth == threadWidth;

  @override
  int get hashCode => Object.hash(
    listLeft,
    listWidth,
    detailLeft,
    detailWidth,
    threadLeft,
    threadWidth,
  );
}

class _PaneGeometryTween extends Tween<_PaneGeometry> {
  _PaneGeometryTween({super.end});

  @override
  _PaneGeometry lerp(double t) => _PaneGeometry.lerp(begin!, end!, t);
}

/// Hides the compact list from input, focus and screen readers while a
/// detail sits above it, even during a transition that still paints it.
class _CoveredList extends StatelessWidget {
  const _CoveredList({required this.covered, required this.child});

  final bool covered;
  final Widget child;

  @override
  Widget build(BuildContext context) => TickerMode(
    enabled: !covered,
    child: IgnorePointer(
      ignoring: covered,
      child: ExcludeFocus(
        excluding: covered,
        child: ExcludeSemantics(
          excluding: covered,
          // A detail's keyboard must not relayout the list it covers. The
          // scope keeps the detail's focus from counting as the list's own.
          child: FocusScope(
            child: CoveredRouteKeyboardInsets(isCovered: covered, child: child),
          ),
        ),
      ),
    ),
  );
}

/// Derives [child]'s [MediaQuery] in a leaf, so the caller's build does not
/// depend on every field (keyboard insets change on each animation frame).
class MediaQueryOverride extends StatelessWidget {
  const MediaQueryOverride({
    required this.transform,
    required this.child,
    super.key,
  });

  final MediaQueryData Function(MediaQueryData) transform;
  final Widget child;

  @override
  Widget build(BuildContext context) =>
      MediaQuery(data: transform(MediaQuery.of(context)), child: child);
}

/// Reports its route as [covering] once the route fully covers the compact
/// list, and withdraws it while the route pushes, pops or follows a back swipe.
class _CoveringDetail extends HookWidget {
  const _CoveringDetail({
    required this.covering,
    required this.mounted,
    required this.builder,
  });

  final ValueNotifier<Route<Object?>?> covering;

  /// Counts mounted details. A popped route unmounts its content only once
  /// its reverse transition has finished.
  final ValueNotifier<int> mounted;
  final WidgetBuilder builder;

  @override
  Widget build(BuildContext context) {
    final route = ModalRoute.of(context);
    final navigator = Navigator.of(context);
    useEffect(() {
      // The count rebuilds an ancestor, which cannot rebuild while this
      // mounts or unmounts.
      WidgetsBinding.instance.addPostFrameCallback((_) => mounted.value++);
      return () =>
          WidgetsBinding.instance.addPostFrameCallback((_) => mounted.value--);
    }, [mounted]);
    useEffect(() {
      if (route == null) return null;
      void sync() {
        final settled =
            route.animation?.status != AnimationStatus.forward &&
            route.animation?.status != AnimationStatus.reverse &&
            !navigator.userGestureInProgress;
        if (settled && route.isCurrent) {
          covering.value = route;
        } else if (covering.value == route) {
          covering.value = null;
        }
      }

      void syncStatus(AnimationStatus _) => sync();
      route.animation?.addStatusListener(syncStatus);
      navigator.userGestureInProgressNotifier.addListener(sync);
      // An immediate route can settle before these listeners attach.
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (context.mounted) sync();
      });
      return () {
        route.animation?.removeStatusListener(syncStatus);
        navigator.userGestureInProgressNotifier.removeListener(sync);
      };
    }, [route, navigator]);
    return builder(context);
  }
}

/// Keeps the route identity stable across layout changes. Sidebar selections
/// are immediate; compact navigation retains the platform push and back swipe.
class _ConversationDetailPage extends MaterialPage<void> {
  const _ConversationDetailPage({
    required this.animate,
    required super.child,
    super.key,
  });

  final bool animate;

  @override
  Route<void> createRoute(BuildContext context) =>
      _ConversationDetailRoute(this);
}

class _ConversationDetailRoute extends MaterialPageRoute<void> {
  _ConversationDetailRoute(_ConversationDetailPage page)
    : super(builder: (_) => page.child, settings: page);

  _ConversationDetailPage get _page => settings as _ConversationDetailPage;

  @override
  Widget buildContent(BuildContext context) => _page.child;

  @override
  Duration get transitionDuration =>
      _page.animate ? super.transitionDuration : Duration.zero;

  @override
  Duration get reverseTransitionDuration =>
      _page.animate ? super.reverseTransitionDuration : Duration.zero;

  @override
  Widget buildTransitions(
    BuildContext context,
    Animation<double> animation,
    Animation<double> secondaryAnimation,
    Widget child,
  ) => super.buildTransitions(
    context,
    _page.animate ? animation : kAlwaysCompleteAnimation,
    secondaryAnimation,
    child,
  );
}

/// Opens a result in its workspace's detail pane, or an ordinary pushed route.
Future<void> openConversationPane(
  BuildContext context,
  String selectionId,
  WidgetBuilder builder, {
  bool reopen = false,
}) async {
  final pane = ConversationPaneScope.maybeOf(context);
  if (pane != null) {
    pane.open(selectionId, builder, reopen: reopen);
  } else {
    await Navigator.of(context).push(MaterialPageRoute<void>(builder: builder));
  }
}

/// Opens a thread beside its conversation on a tablet, or pushes it.
///
/// A thread opened from inside a thread pane stacks in that pane, and phones,
/// Duo and destinations without a pane keep an ordinary pushed route.
Future<void> openThreadPane(BuildContext context, WidgetBuilder builder) async {
  final pane = ConversationPaneScope.maybeOf(context);
  if (pane != null &&
      pane.selectedId != null &&
      !ConversationThreadScope.isInside(context) &&
      // Only a tablet layout's conversation column carries the sidebar scope.
      context.getInheritedWidgetOfExactType<ConversationSidebarScope>() !=
          null) {
    pane.openThread(builder);
    return;
  }
  await Navigator.of(context).push(MaterialPageRoute<void>(builder: builder));
}

/// A list/detail workspace for destinations such as Search and Inbox.
class AdaptiveConversationDestination extends HookConsumerWidget {
  const AdaptiveConversationDestination({
    required this.child,
    this.emptyLabel = 'Select a conversation',
    this.listWidthFraction,
    this.controller,
    this.persistentSelection = false,
    this.listToolbarSlots = 0,
    this.preferredListWidth,
    super.key,
  });

  /// Optional list share of the usable width; defaults to the compact sidebar.
  final ConversationPaneController? controller;
  final bool persistentSelection;

  /// Rail slots reserved for workspace controls that stay active in wide mode.
  final int listToolbarSlots;

  /// Preferred sidebar width, capped to preserve room for its detail pane.
  final double? preferredListWidth;
  final double? listWidthFraction;
  final String emptyLabel;
  final Widget child;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final ownedController = useMemoized(ConversationPaneController.new);
    useEffect(() => ownedController.dispose, [ownedController]);
    final controller = this.controller ?? ownedController;
    final strip = FoldGeometry.of(context).strip;
    return ColoredBox(
      color: context.colors.surface,
      child: DuoWorkspaceScaffold(
        navigationBuilder: (_, _) => const SizedBox.shrink(),
        body: MediaQueryOverride(
          transform: (media) => media.removePadding(
            removeLeft: strip?.side == FoldStripSide.left,
            removeRight: strip?.side == FoldStripSide.right,
          ),
          child: AdaptiveConversationPane(
            active: DuoToolbarScope.maybeOf(context)?.active ?? true,
            controller: controller,
            persistentSelection: persistentSelection,
            listToolbarSlots: listToolbarSlots,
            emptyLabel: emptyLabel,
            listWidthFraction: listWidthFraction,
            preferredListWidth: preferredListWidth,
            strip: strip,
            list: child,
          ),
        ),
      ),
    );
  }
}
