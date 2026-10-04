import 'package:flutter/material.dart';
import 'package:flutter/foundation.dart';
import 'ios_navigation_bar.dart';
import 'package:flutter_hooks/flutter_hooks.dart';

import '../theme/theme.dart';
import 'directional_transition_scope.dart';
import 'frosted_app_bar.dart';
import 'frosted_scroll_under_scope.dart';

/// A convenience [Scaffold] that overlays a [FrostedAppBar] on top of its body.
///
/// The body is rendered full-bleed inside a [Stack] with the frosted app bar
/// floating above it. The body is responsible for adding its own top spacing
/// using [frostedAppBarHeight] so content starts below the bar.
class FrostedScaffold extends HookWidget {
  /// The frosted app bar displayed at the top of the screen.
  final FrostedAppBar appBar;

  /// The primary content of the scaffold. Must handle its own top spacing
  /// using [frostedAppBarHeight] — the scaffold does NOT add automatic padding.
  final Widget body;

  /// Optional floating action button, passed through to [Scaffold].
  final Widget? floatingActionButton;

  /// Whether the body should resize when the on-screen keyboard appears.
  final bool? resizeToAvoidBottomInset;

  /// Optional scaffold background, useful when a parent supplies a shared
  /// surface behind this page.
  final Color? backgroundColor;

  /// A fixed gradient painted behind the app bar and scrolling body.
  final Gradient? backgroundGradient;

  /// Whether this settings-style page should invert the canvas and container
  /// surface roles.
  final bool useUtilitySurfaceTheme;

  /// Reserves the native large title above fixed controls.
  final bool nativePinnedBody;

  const FrostedScaffold({
    super.key,
    required this.appBar,
    required this.body,
    this.floatingActionButton,
    this.resizeToAvoidBottomInset,
    this.backgroundColor,
    this.backgroundGradient,
    this.useUtilitySurfaceTheme = false,
    this.nativePinnedBody = false,
  });

  @override
  Widget build(BuildContext context) {
    final scrollOffset = useValueNotifier(0.0);
    final pinnedController = useScrollController(keepScrollOffset: false);
    // A keyed page/tab owns both coordinated positions and their restoration
    // bucket, so a new timeline cannot restore a stale inner offset.
    final pinnedStorage = useMemoized(PageStorageBucket.new);
    final pinnedKey = useMemoized(GlobalKey<NestedScrollViewState>.new);
    final collapseExtent = appBar.nativeLargeTitle
        ? IosNavigationMetrics.of(context).largeTitleHeight
        : 0.0;
    final previousExtent = useRef(collapseExtent);
    // Redistribute consumed distance after native metric changes without
    // shifting the visible content or expanding a deeply scrolled title.
    useEffect(() {
      final old = previousExtent.value;
      previousExtent.value = collapseExtent;
      final inner = pinnedKey.currentState?.innerController;
      if (old != collapseExtent &&
          pinnedController.hasClients &&
          pinnedController.offset >= old &&
          inner != null &&
          inner.hasClients &&
          inner.offset > 0) {
        WidgetsBinding.instance.addPostFrameCallback((_) {
          if (!pinnedController.hasClients || !inner.hasClients) return;
          final total = pinnedController.offset + inner.offset;
          pinnedController.jumpTo(collapseExtent.clamp(0.0, total));
          inner.jumpTo((total - collapseExtent).clamp(0.0, double.infinity));
        });
      }
      return null;
    }, [collapseExtent]);
    final nativePinned =
        nativePinnedBody && defaultTargetPlatform == TargetPlatform.iOS;
    useEffect(() {
      void update() {
        if (pinnedController.hasClients) {
          final inner = pinnedKey.currentState?.innerController;
          scrollOffset.value =
              (pinnedController.offset +
                      (inner != null && inner.hasClients ? inner.offset : 0))
                  .clamp(0.0, double.infinity);
        }
      }

      if (!nativePinned) return null;
      pinnedController.addListener(update);
      return () => pinnedController.removeListener(update);
    }, [nativePinned, pinnedController]);
    final isScrolledUnder = useState(false);
    final pendingScrolledUnder = useRef<bool?>(null);
    final scrollUpdateScheduled = useRef(false);

    void updateScrollUnder(bool next) {
      if (scrollUpdateScheduled.value) {
        pendingScrolledUnder.value =
            (pendingScrolledUnder.value ?? false) || next;
        return;
      }
      pendingScrolledUnder.value = next;
      scrollUpdateScheduled.value = true;
      WidgetsBinding.instance.addPostFrameCallback((_) {
        scrollUpdateScheduled.value = false;
        final pending = pendingScrolledUnder.value;
        pendingScrolledUnder.value = null;
        if (!context.mounted ||
            pending == null ||
            pending == isScrolledUnder.value) {
          return;
        }
        isScrolledUnder.value = pending;
      });
    }

    final observedBody = NotificationListener<ScrollNotification>(
      onNotification: (notification) {
        if (nativePinned &&
            notification.metrics.axis == Axis.vertical &&
            pinnedController.hasClients) {
          final inner = pinnedKey.currentState?.innerController;
          scrollOffset.value =
              (pinnedController.offset +
                      (inner != null && inner.hasClients ? inner.offset : 0))
                  .clamp(0.0, double.infinity);
        }
        if (!nativePinned &&
            notification.depth == 0 &&
            notification.metrics.axis == Axis.vertical) {
          // Keep real depth so a later UIKit metrics update (Dynamic Type or
          // rotation) can apply its new collapse range without another scroll.
          final next = notification.metrics.pixels.clamp(0.0, double.infinity);
          if ((scrollOffset.value - next).abs() > 0.1) {
            scrollOffset.value = next;
          }
        }
        if (notification.depth != 0 ||
            notification.metrics.axis != Axis.vertical ||
            (notification is! ScrollUpdateNotification &&
                notification is! OverscrollNotification)) {
          return false;
        }
        final next = notification.metrics.extentBefore > 0.5;
        if (next != isScrolledUnder.value) updateScrollUnder(next);
        return false;
      },
      child: body,
    );
    final scaffold = IosNavigationScrollScope(
      offset: scrollOffset,
      child: Scaffold(
        backgroundColor: backgroundColor,
        resizeToAvoidBottomInset: resizeToAvoidBottomInset,
        floatingActionButton: floatingActionButton,
        body: FrostedScrollUnderScope(
          isScrolledUnder: isScrolledUnder.value,
          child: Stack(
            children: _stackChildren(
              observedBody,
              pinnedController,
              pinnedStorage,
              pinnedKey,
            ),
          ),
        ),
      ),
    );
    if (!useUtilitySurfaceTheme) return scaffold;
    return Theme(
      data: utilitySurfaceThemeData(Theme.of(context)),
      child: scaffold,
    );
  }

  List<Widget> _stackChildren(
    Widget observedBody,
    ScrollController pinnedController,
    PageStorageBucket pinnedStorage,
    GlobalKey<NestedScrollViewState> pinnedKey,
  ) {
    final backdrop = backgroundGradient == null
        ? const <Widget>[]
        : [
            Positioned.fill(
              child: _PinnedGradientBackground(gradient: backgroundGradient!),
            ),
          ];
    final bodyMotion = DirectionalTransitionMotion(
      transformKey: const ValueKey(
        'frosted-scaffold-body-transition-transform',
      ),
      opacityKey: const ValueKey('frosted-scaffold-body-transition-opacity'),
      child: nativePinnedBody && defaultTargetPlatform == TargetPlatform.iOS
          ? PageStorage(
              bucket: pinnedStorage,
              child: NestedScrollView(
                key: pinnedKey,
                controller: pinnedController,
                headerSliverBuilder: (context, innerScrolled) => [
                  SliverToBoxAdapter(
                    child: SizedBox(
                      height: appBar.nativeLargeTitle
                          ? IosNavigationMetrics.of(context).largeTitleHeight
                          : 0,
                    ),
                  ),
                ],
                body: observedBody,
              ),
            )
          : observedBody,
    );
    // The bar must be painted after the scrollable sheet: [BackdropFilter]
    // only samples pixels that were already painted behind it. This is the
    // same composition as channel navigation, so top-level headers blur their
    // content rather than only the fixed gradient.
    return [...backdrop, bodyMotion, appBar];
  }
}

class _PinnedGradientBackground extends StatelessWidget {
  final Gradient gradient;

  const _PinnedGradientBackground({required this.gradient});

  @override
  Widget build(BuildContext context) => DecoratedBox(
    key: const ValueKey('frosted-scaffold-pinned-gradient'),
    decoration: BoxDecoration(gradient: gradient),
  );
}
