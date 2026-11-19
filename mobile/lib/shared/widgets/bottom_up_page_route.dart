import 'ios_composition_layer.dart';
import '../theme/theme.dart';

import 'package:flutter/cupertino.dart';
import 'package:flutter/foundation.dart';

/// Presents a full-screen utility page with the same bottom-up motion as Search.
class BottomUpPageRoute<T> extends PageRouteBuilder<T> {
  BottomUpPageRoute({
    required WidgetBuilder builder,
    required bool reducedMotion,
  }) : super(
         opaque: defaultTargetPlatform == TargetPlatform.iOS,
         allowSnapshotting: false,
         transitionDuration: reducedMotion
             ? Duration.zero
             : defaultTargetPlatform == TargetPlatform.iOS
             ? CupertinoRouteTransitionMixin.kTransitionDuration
             : const Duration(milliseconds: 260),
         reverseTransitionDuration: reducedMotion
             ? Duration.zero
             : defaultTargetPlatform == TargetPlatform.iOS
             ? CupertinoRouteTransitionMixin.kTransitionDuration
             : const Duration(milliseconds: 220),
         pageBuilder: (context, animation, secondaryAnimation) =>
             IosCompositionLayer(
               color: context.colors.surface,
               child: builder(context),
             ),
         transitionsBuilder: (context, animation, secondaryAnimation, child) =>
             reducedMotion
             ? child
             : defaultTargetPlatform == TargetPlatform.iOS
             ? CupertinoFullscreenDialogTransition(
                 primaryRouteAnimation: animation,
                 secondaryRouteAnimation: secondaryAnimation,
                 linearTransition: false,
                 child: child,
               )
             : SlideTransition(
                 position: animation.drive(
                   Tween(
                     begin: const Offset(0, 1),
                     end: Offset.zero,
                   ).chain(CurveTween(curve: Curves.easeOutCubic)),
                 ),
                 child: child,
               ),
       );
  static final _coveringSheets = Expando<ValueNotifier<int>>();

  /// Tracks sheet coverage through disposal, including the reverse transition.
  static ValueListenable<int> sheetsAbove(ModalRoute<dynamic> route) =>
      _coveringSheets[route] ??= ValueNotifier(0);

  ModalRoute<dynamic>? _coveredRoute;

  void _setCoveredRoute(ModalRoute<dynamic>? route) {
    if (identical(route, _coveredRoute)) return;
    final previous = _coveredRoute;
    if (previous != null) _coveringSheets[previous]!.value--;
    _coveredRoute = route;
    if (route != null) {
      (_coveringSheets[route] ??= ValueNotifier(0)).value++;
    }
  }

  @override
  void didChangePrevious(Route<dynamic>? previousRoute) {
    super.didChangePrevious(previousRoute);
    _setCoveredRoute(previousRoute is ModalRoute ? previousRoute : null);
  }

  @override
  void dispose() {
    _setCoveredRoute(null);
    super.dispose();
  }
}
