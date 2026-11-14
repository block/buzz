import 'package:flutter/material.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:duo_kit/duo_kit.dart';

import 'duo_toolbar.dart';
import 'app_route_observer.dart';
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
  String? selectedId;
  WidgetBuilder? detailBuilder;
  bool animateSelection = true;
  int _request = 0;
  // Deep-link requests may target the same item after its nested route popped.
  // A fresh page identity reruns initialization without clearing the selection
  // when Navigator removes the previous page.
  LocalKey get selectionKey => ValueKey((selectedId, _request));

  void open(
    String id,
    WidgetBuilder builder, {
    bool animate = true,
    bool reopen = false,
  }) {
    if (id == selectedId && !reopen) return;
    if (reopen) _request++;
    selectedId = id;
    detailBuilder = builder;
    animateSelection = animate;
    notifyListeners();
  }

  void clear() {
    if (selectedId == null) return;
    selectedId = null;
    detailBuilder = null;
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
            final wide = usesConversationPanes(context, strip: strip);
            final book = wide ? bookDivisionOf(context) : null;
            final selected = controller.selectedId;
            if (ownerRoute != null) {
              _compactPaneNavigators[ownerRoute] =
                  active && !wide && selected != null ? navigator : null;
            }
            // Pane surfaces meet at the fold so their header backdrops remain
            // continuous. Content insets below keep controls out of its bounds.
            final listWidth = book != null
                ? book.center.dx - leftInset
                : wide
                ? listWidthFraction == null
                      ? preferredListWidth?.clamp(
                              0.0,
                              (contentWidth - 1) / 2,
                            ) ??
                            (contentWidth / 3).clamp(280.0, 360.0)
                      : (contentWidth - 1) * listWidthFraction!
                : constraints.maxWidth;
            final detailLeft =
                book?.center.dx ?? (wide ? listWidth + leftInset + 1 : 0.0);
            return TweenAnimationBuilder<Offset>(
              tween: Tween(end: Offset(listWidth, detailLeft)),
              duration: wide && !MediaQuery.disableAnimationsOf(context)
                  ? const Duration(milliseconds: 240)
                  : Duration.zero,
              curve: Curves.easeInOutCubic,
              builder: (context, geometry, _) {
                final listWidth = geometry.dx;
                final detailLeft = geometry.dy;
                final detailWidth = constraints.maxWidth - detailLeft;
                return Stack(
                  fit: StackFit.expand,
                  children: [
                    Positioned(
                      left: 0,
                      top: 0,
                      bottom: 0,
                      width: wide ? listWidth + leftInset : listWidth,
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
                          covered: !wide && selected != null,
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
                    Positioned(
                      left: detailLeft,
                      top: 0,
                      bottom: 0,
                      width: detailWidth,
                      child: Offstage(
                        offstage: !wide && selected == null,
                        child: TickerMode(
                          enabled: wide || selected != null,
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
                                  (ModalRoute.isCurrentOf(context) ?? true),
                              child: Semantics(
                                container: true,
                                explicitChildNodes: true,
                                child: PopScope<Object?>(
                                  canPop:
                                      !active ||
                                      selected == null ||
                                      (wide && persistentSelection),
                                  onPopInvokedWithResult: (didPop, result) {
                                    if (!didPop &&
                                        active &&
                                        selected != null &&
                                        !(wide && persistentSelection)) {
                                      navigator.currentState!.maybePop(result);
                                    }
                                  },
                                  child: ExcludeFocus(
                                    excluding: selected == null,
                                    child: NavigatorPopHandler<Object?>(
                                      enabled: wide && persistentSelection,
                                      onPopWithResult: (result) =>
                                          navigator.currentState!.pop(result),
                                      child: Navigator(
                                        requestFocus: selected != null,
                                        key: navigator,
                                        observers: [routeObserver],
                                        pages: [
                                          if (!(wide &&
                                              persistentSelection &&
                                              selected != null))
                                            MaterialPage<void>(
                                              key: const ValueKey(
                                                'conversation-placeholder',
                                              ),
                                              // Compact panes show the list
                                              // through this page while a
                                              // detail pushes or pops.
                                              child: !wide
                                                  ? const SizedBox.shrink()
                                                  : Material(
                                                      color: context
                                                          .colors
                                                          .surface,
                                                      child: Center(
                                                        child:
                                                            emptyContent ??
                                                            Text(
                                                              emptyLabel,
                                                              textAlign:
                                                                  TextAlign
                                                                      .center,
                                                              style: Theme.of(context)
                                                                  .textTheme
                                                                  .bodyLarge
                                                                  ?.copyWith(
                                                                    color: Theme.of(
                                                                      context,
                                                                    ).colorScheme.onSurfaceVariant,
                                                                    decoration:
                                                                        TextDecoration
                                                                            .none,
                                                                  ),
                                                            ),
                                                      ),
                                                    ),
                                            ),
                                          if (selected != null)
                                            if (controller.animateSelection)
                                              MaterialPage<void>(
                                                key: controller.selectionKey,
                                                child: _CoveringDetail(
                                                  covering: coveringDetail,
                                                  builder:
                                                      controller.detailBuilder!,
                                                ),
                                              )
                                            else
                                              _ImmediateDetailPage(
                                                key: controller.selectionKey,
                                                builder: (_) => _CoveringDetail(
                                                  covering: coveringDetail,
                                                  builder:
                                                      controller.detailBuilder!,
                                                ),
                                              ),
                                        ],
                                        onDidRemovePage: (page) {
                                          if (page.key ==
                                              controller.selectionKey) {
                                            controller.clear();
                                          }
                                        },
                                      ),
                                    ),
                                  ),
                                ),
                              ),
                            ),
                          ),
                        ),
                      ),
                    ),
                    if (wide)
                      Positioned(
                        left: listWidth + leftInset,
                        top: 0,
                        bottom: 0,
                        width: 1,
                        child: IgnorePointer(
                          child: ColoredBox(
                            color: context.colors.outlineVariant,
                          ),
                        ),
                      ),
                  ],
                );
              },
            );
          },
        ),
      ),
    );
  }
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
  const _CoveringDetail({required this.covering, required this.builder});

  final ValueNotifier<Route<Object?>?> covering;
  final WidgetBuilder builder;

  @override
  Widget build(BuildContext context) {
    final route = ModalRoute.of(context);
    final navigator = Navigator.of(context);
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

/// Allows a detail with its own shared-element motion to own the transition.
class _ImmediateDetailPage extends Page<void> {
  const _ImmediateDetailPage({required this.builder, super.key});
  final WidgetBuilder builder;

  @override
  Route<void> createRoute(BuildContext context) => PageRouteBuilder<void>(
    settings: this,
    transitionDuration: Duration.zero,
    reverseTransitionDuration: Duration.zero,
    pageBuilder: (context, _, _) => builder(context),
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
