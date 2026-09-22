import 'package:flutter/material.dart';

/// A route for destinations that provide their own in-page transition.
PageRoute<T> immediatePageRoute<T>({required WidgetBuilder builder}) =>
    PageRouteBuilder<T>(
      transitionDuration: Duration.zero,
      reverseTransitionDuration: Duration.zero,
      pageBuilder: (context, _, _) => builder(context),
      transitionsBuilder: (_, _, _, child) => child,
    );
