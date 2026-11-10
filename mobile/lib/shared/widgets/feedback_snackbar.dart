import 'dart:async';

import 'package:flutter/material.dart';

import '../toast_haptic.dart';

export '../toast_haptic.dart' show ToastKind;

/// Applies the shared feedback pulse to the remaining actionable snackbars.
extension FeedbackSnackBar on ScaffoldMessengerState {
  /// Shows the original snackbar, preserving its actions, queue, and lifetime.
  ScaffoldFeatureController<SnackBar, SnackBarClosedReason>
  showFeedbackSnackBar(SnackBar snackBar, {required ToastKind kind}) {
    final controller = showSnackBar(snackBar);
    unawaited(toastHaptic(kind));
    return controller;
  }
}
