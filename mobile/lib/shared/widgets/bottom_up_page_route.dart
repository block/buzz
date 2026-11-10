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
             builder(context),
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
}
