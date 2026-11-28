import 'dart:ui' show PointerDeviceKind;

import 'package:flutter/material.dart';

/// App-wide scroll behavior that lets a mouse drag scroll lists.
///
/// Flutter's default [MaterialScrollBehavior] only starts a drag for
/// touch-like pointers (finger, stylus, trackpad). On an Android tablet with
/// a mouse attached, the platform treats a mouse drag like a finger drag in
/// every other app, so Buzz has to opt in or its lists feel stuck. Wheel
/// scrolling is unaffected either way.
class AppScrollBehavior extends MaterialScrollBehavior {
  const AppScrollBehavior();

  @override
  Set<PointerDeviceKind> get dragDevices => {
    ...super.dragDevices,
    PointerDeviceKind.mouse,
  };
}
