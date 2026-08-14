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
  /// Runs after Android metrics remain quiet for [androidSettleDelay], or
  /// immediately for each metrics change on other platforms.
  final VoidCallback onMetricsSettled;

  /// The quiet period used to coalesce Android IME animation metrics.
  final Duration androidSettleDelay;
  Timer? _androidTimer;

  /// Creates an observer that coalesces Android IME metric updates.
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

  /// Cancels any pending Android metrics-settlement callback.
  void dispose() {
    _androidTimer?.cancel();
    _androidTimer = null;
  }
}
