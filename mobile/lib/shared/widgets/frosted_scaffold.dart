import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';

import 'frosted_app_bar.dart';
import 'frosted_header_scroll_state.dart';

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

  const FrostedScaffold({
    super.key,
    required this.appBar,
    required this.body,
    this.floatingActionButton,
    this.resizeToAvoidBottomInset,
    this.backgroundColor,
  });

  @override
  Widget build(BuildContext context) {
    // This is deliberately screen-size based rather than platform based: a
    // compact window on iPad keeps the mobile frosted chrome, while the roomy
    // iPad layout gets a flush desktop-style header.
    final useIpadHeaderTreatment =
        MediaQuery.sizeOf(context).shortestSide >= 600;
    final headerIsScrolled = useState(false);

    return NotificationListener<ScrollNotification>(
      onNotification: (notification) {
        if (!useIpadHeaderTreatment ||
            notification.metrics.axis != Axis.vertical) {
          return false;
        }
        final isScrolled = notification.metrics.pixels > 0;
        if (headerIsScrolled.value != isScrolled) {
          headerIsScrolled.value = isScrolled;
        }
        return false;
      },
      child: FrostedHeaderScrollState(
        notifier: headerIsScrolled,
        child: Scaffold(
          backgroundColor: backgroundColor,
          resizeToAvoidBottomInset: resizeToAvoidBottomInset,
          floatingActionButton: floatingActionButton,
          body: Stack(children: [body, appBar]),
        ),
      ),
    );
  }
}
