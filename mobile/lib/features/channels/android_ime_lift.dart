import 'dart:math' show max;

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';

/// Android keeps the message viewport fixed while the IME animates. Only this
/// small wrapper follows the frame-by-frame inset; timelines apply the final
/// inset once metrics settle.
class AndroidImeLift extends StatelessWidget {
  /// The composer subtree that follows the animated Android IME inset.
  final Widget child;

  /// Creates a wrapper that lifts [child] without resizing its surrounding
  /// message viewport on Android.
  const AndroidImeLift({super.key, required this.child});

  @override
  Widget build(BuildContext context) {
    if (!usesFixedAndroidImeViewport) return child;
    final imeBottom = MediaQuery.viewInsetsOf(context).bottom;
    final systemBottom = MediaQuery.viewPaddingOf(context).bottom;
    return Padding(
      // The composer already reserves [systemBottom]. Android's IME inset
      // includes that navigation area, so lifting by the full value leaves a
      // second safe-area gap above the keyboard.
      padding: EdgeInsets.only(bottom: max(0, imeBottom - systemBottom)),
      child: child,
    );
  }
}

/// Whether channel and thread scaffolds should keep a fixed viewport while
/// Android IME insets animate, with their composer lifted independently.
bool get usesFixedAndroidImeViewport =>
    defaultTargetPlatform == TargetPlatform.android;
