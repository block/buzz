import 'package:flutter/material.dart';

import 'directional_transition_scope.dart';
import 'frosted_app_bar.dart';

/// A convenience [Scaffold] that overlays a [FrostedAppBar] on top of its body.
///
/// The body is rendered full-bleed inside a [Stack] with the frosted app bar
/// floating above it. The body is responsible for adding its own top spacing
/// using [frostedAppBarHeight] so content starts below the bar.
class FrostedScaffold extends StatelessWidget {
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

  const FrostedScaffold({
    super.key,
    required this.appBar,
    required this.body,
    this.floatingActionButton,
    this.resizeToAvoidBottomInset,
    this.backgroundColor,
    this.backgroundGradient,
  });

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      backgroundColor: backgroundColor,
      resizeToAvoidBottomInset: resizeToAvoidBottomInset,
      floatingActionButton: floatingActionButton,
      body: Stack(children: _stackChildren()),
    );
  }

  List<Widget> _stackChildren() {
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
      child: body,
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
