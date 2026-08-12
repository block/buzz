import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter/widgets.dart';

/// How long Android viewport metrics must stay quiet before layout correction.
const androidImeMetricsSettleDelay = Duration(milliseconds: 120);

/// Coalesces Android's frame-by-frame IME metrics into one settled callback.
///
/// iOS continues to receive callbacks immediately. Android sends viewport
/// metrics throughout the keyboard animation; doing list realignment for each
/// delivery competes with the IME transition and can drop frames.
class ImeMetricsSettleObserver with WidgetsBindingObserver {
  final VoidCallback onMetricsSettled;
  final Duration androidSettleDelay;
  Timer? _androidTimer;

  ImeMetricsSettleObserver({
    required this.onMetricsSettled,
    this.androidSettleDelay = androidImeMetricsSettleDelay,
  });

  @override
  void didChangeMetrics() {
    if (defaultTargetPlatform != TargetPlatform.android) {
      onMetricsSettled();
      return;
    }
    _androidTimer?.cancel();
    _androidTimer = Timer(androidSettleDelay, onMetricsSettled);
  }

  void dispose() {
    _androidTimer?.cancel();
    _androidTimer = null;
  }
}
