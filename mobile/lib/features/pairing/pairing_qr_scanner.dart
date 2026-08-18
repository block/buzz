import 'dart:async';
import 'dart:math' as math;
import 'dart:ui' show lerpDouble;

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';
import 'package:mobile_scanner/mobile_scanner.dart';

import '../../shared/theme/theme.dart';

part 'pairing_qr_scanner/dynamic_island_portal.dart';
part 'pairing_qr_scanner/fallback_scanner.dart';
part 'pairing_qr_scanner/scanner_camera.dart';

const _qrScannerPlatformChannel = MethodChannel('buzz/qr_scanner');

/// Returns whether the current iPhone should use the Dynamic Island scanner.
///
/// Android, iPad, and iPhones without a Dynamic Island always use the standard
/// full-screen scanner.
Future<bool> usesDynamicIslandQrScannerPortal() async {
  if (defaultTargetPlatform != TargetPlatform.iOS) {
    return false;
  }

  try {
    return await _qrScannerPlatformChannel.invokeMethod<bool>(
          'usesDynamicIslandQrScannerPortal',
        ) ??
        false;
  } on PlatformException {
    return false;
  } on MissingPluginException {
    return false;
  }
}

/// Opens the Dynamic Island QR scanner portal.
///
/// Callers determine device support with
/// [usesDynamicIslandQrScannerPortal]. Standard devices reveal the camera
/// behind their existing app surface with [FallbackPairingQrScanner].
Future<String?> showDynamicIslandPairingQrScanner(BuildContext context) {
  return Navigator.of(context).push<String>(
    PageRouteBuilder<String>(
      opaque: false,
      barrierColor: Colors.transparent,
      barrierDismissible: false,
      transitionDuration: Duration.zero,
      reverseTransitionDuration: Duration.zero,
      pageBuilder: (_, _, _) => const _DynamicIslandQrScannerPortal(),
    ),
  );
}

Future<void> _setDynamicIslandScannerStatusBarHidden(bool hidden) async {
  try {
    await _qrScannerPlatformChannel.invokeMethod<void>(
      'setDynamicIslandScannerStatusBarHidden',
      hidden,
    );
  } on PlatformException {
    // The scanner still works if the native status-bar bridge is unavailable.
  } on MissingPluginException {
    // Widget tests and non-iOS embedders do not register the bridge.
  }
}

Future<void> _performDynamicIslandQrScanSuccessHaptic() async {
  try {
    await _qrScannerPlatformChannel.invokeMethod<void>(
      'performDynamicIslandQrScanSuccessHaptic',
    );
  } on PlatformException {
    // Scanning should still complete if haptics are unavailable.
  } on MissingPluginException {
    // Widget tests and non-iOS embedders do not register the bridge.
  }
}

String? _firstScannedValue(BarcodeCapture capture) {
  for (final barcode in capture.barcodes) {
    final value = barcode.rawValue;
    if (value != null && value.isNotEmpty) {
      return value;
    }
  }
  return null;
}

/// Geometry for the iPhone Dynamic Island scanner portal.
///
/// The collapsed and expanded frames share one top edge. This makes the camera
/// grow down from the hardware cutout instead of scaling around the center of
/// its final square.
@visibleForTesting
class DynamicIslandQrScannerGeometry {
  /// Creates geometry for a viewport and its top safe-area inset.
  const DynamicIslandQrScannerGeometry({
    required this.viewport,
    required this.safeAreaTop,
  });

  /// The full logical-pixel size available to the portal route.
  final Size viewport;

  /// The top safe-area inset reported by iOS.
  final double safeAreaTop;

  static const _collapsedWidth = 120.0;
  static const _collapsedHeight = 36.0;
  static const _fallbackTop = 11.0;
  static const _referenceSafeAreaTop = 59.0;
  static const _edgeMargin = 15.0;
  static const _expandedCornerRadius = 40.0;
  static const _scannerFadeStart = 0.18;
  static const _introLabelFadeEnd = 0.42;

  /// The physical-island-aligned top edge used throughout the morph.
  double get top =>
      _fallbackTop + math.max(0, safeAreaTop - _referenceSafeAreaTop);

  /// The portal frame before the opening animation starts.
  Rect get collapsedFrame => Rect.fromLTWH(
    (viewport.width - _collapsedWidth) / 2,
    top,
    _collapsedWidth,
    _collapsedHeight,
  );

  /// The square camera frame after the opening animation completes.
  Rect get expandedFrame {
    final maxHeight = math.max(
      _collapsedHeight,
      viewport.height - top - _edgeMargin,
    );
    final size = math.min(viewport.width - (_edgeMargin * 2), maxHeight);
    return Rect.fromLTWH(_edgeMargin, top, size, size);
  }

  /// Returns the top-anchored portal frame at [progress].
  Rect frameAt(double progress) {
    final t = progress.clamp(0.0, 1.0);
    final start = collapsedFrame;
    final end = expandedFrame;
    return Rect.fromLTRB(
      lerpDouble(start.left, end.left, t)!,
      top,
      lerpDouble(start.right, end.right, t)!,
      lerpDouble(start.bottom, end.bottom, t)!,
    );
  }

  /// Returns the portal corner radius at [progress].
  double cornerRadiusAt(double progress) {
    final t = progress.clamp(0.0, 1.0);
    return lerpDouble(_collapsedHeight / 2, _expandedCornerRadius, t)!;
  }

  /// Returns the camera opacity after its delayed entrance.
  double scannerOpacityAt(double progress) {
    return ((progress - _scannerFadeStart) / (1 - _scannerFadeStart)).clamp(
      0.0,
      1.0,
    );
  }

  /// Returns the opacity of the compact prompt inside the island pill.
  double introLabelOpacityAt(double progress) {
    return (1 - (progress / _introLabelFadeEnd)).clamp(0.0, 1.0);
  }
}
