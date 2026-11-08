import 'package:flutter/material.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:duo_kit/duo_kit.dart';

import 'duo_toolbar.dart';
import 'app_route_observer.dart';
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
  if (defaultTargetPlatform == TargetPlatform.iOS &&
      size.height >= size.width) {
    return false;
  }
  final bookDivision = bookDivisionOf(context);
  // A landscape phone is wide enough for the breakpoint, but not for two
  // usable panes. Fold geometry keeps Duo eligible in its supported poses.
  if (defaultTargetPlatform == TargetPlatform.iOS &&
      size.shortestSide < 600 &&
      rail == null &&
      bookDivision == null) {
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

/// Selection belongs to the workspace, rather than to a particular display pose.
class ConversationPaneController extends ChangeNotifier {
  String? selectedId;
  WidgetBuilder? detailBuilder;
  bool animateSelection = true;

  void open(String id, WidgetBuilder builder, {bool animate = true}) {
    if (id == selectedId) return;
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
  const ConversationPaneScope({
    required ConversationPaneController controller,
    required super.child,
    this.active = true,
    super.key,
  }) : super(notifier: controller);

  final bool active;

  static bool isActiveOf(BuildContext context) =>
      context
          .dependOnInheritedWidgetOfExactType<ConversationPaneScope>()
          ?.active ??
      true;

  @override
  bool updateShouldNotify(ConversationPaneScope oldWidget) =>
      active != oldWidget.active || super.updateShouldNotify(oldWidget);

  static ConversationPaneController? maybeOf(BuildContext context) => context
      .dependOnInheritedWidgetOfExactType<ConversationPaneScope>()
      ?.notifier;
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
    super.key,
  });

  final bool persistentSelection;

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
            final listWidth = book != null
                ? book.left - leftInset
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
                book?.right ?? (wide ? listWidth + leftInset + 1 : 0.0);
            final media = MediaQuery.of(context);
            return TweenAnimationBuilder<Offset>(
              tween: Tween(end: Offset(listWidth, detailLeft)),
              duration: wide && !media.disableAnimations
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
                          child: MediaQuery(
                            data: media.copyWith(
                              size: Size(listWidth, constraints.maxHeight),
                            ),
                            child: DuoToolbarScope(
                              strip: strip,
                              contentInsets: EdgeInsets.only(
                                left: leftInset,
                                right: wide ? 0 : rightInset,
                              ),
                              active:
                                  active &&
                                  (selected == null ||
                                      (wide && persistentSelection)) &&
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
                          child: MediaQuery(
                            data: media.copyWith(
                              size: Size(detailWidth, constraints.maxHeight),
                            ),
                            child: DuoToolbarScope(
                              strip: strip,
                              contentInsets: EdgeInsets.only(
                                left: wide ? 0 : leftInset,
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
                                                key: ValueKey<String?>(
                                                  selected,
                                                ),
                                                child: _CoveringDetail(
                                                  covering: coveringDetail,
                                                  builder:
                                                      controller.detailBuilder!,
                                                ),
                                              )
                                            else
                                              _ImmediateDetailPage(
                                                key: ValueKey<String?>(
                                                  selected,
                                                ),
                                                builder: (_) => _CoveringDetail(
                                                  covering: coveringDetail,
                                                  builder:
                                                      controller.detailBuilder!,
                                                ),
                                              ),
                                        ],
                                        onDidRemovePage: (page) {
                                          if (page.key ==
                                              ValueKey(controller.selectedId)) {
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
        child: ExcludeSemantics(excluding: covered, child: child),
      ),
    ),
  );
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
  WidgetBuilder builder,
) async {
  final pane = ConversationPaneScope.maybeOf(context);
  if (pane != null) {
    pane.open(selectionId, builder);
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
    this.preferredListWidth,
    super.key,
  });

  /// Optional list share of the usable width; defaults to the compact sidebar.
  final ConversationPaneController? controller;
  final bool persistentSelection;

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
    final media = MediaQuery.of(context);
    return ColoredBox(
      color: context.colors.surface,
      child: DuoWorkspaceScaffold(
        navigationBuilder: (_, _) => const SizedBox.shrink(),
        body: MediaQuery(
          data: media.removePadding(
            removeLeft: strip?.side == FoldStripSide.left,
            removeRight: strip?.side == FoldStripSide.right,
          ),
          child: AdaptiveConversationPane(
            active: DuoToolbarScope.maybeOf(context)?.active ?? true,
            controller: controller,
            persistentSelection: persistentSelection,
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
