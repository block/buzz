import 'dart:async';

import 'package:flutter/widgets.dart';

/// Carries submitted composer geometry through asynchronous send preparation.
class ComposerSendOrigin {
  static final _zoneKey = Object();

  /// Captures only geometry; no draft or identity data is retained.
  static Rect? capture(GlobalKey key) {
    final box = key.currentContext?.findRenderObject();
    return box is RenderBox && box.hasSize && box.attached
        ? box.localToGlobal(Offset.zero) & box.size
        : null;
  }

  /// Geometry for this send, isolated from other concurrent submissions.
  static Rect? get current => Zone.current[_zoneKey] as Rect?;

  /// Retains the submitted surface until this send creates its local claim.
  static Future<void> run(Rect? origin, Future<void> Function() send) =>
      runZoned(send, zoneValues: {_zoneKey: origin});
}
