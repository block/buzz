import 'dart:async';
import 'dart:math' as math;
import 'dart:ui' show lerpDouble;

import 'package:duo_kit/duo_kit.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/physics.dart';
import 'package:flutter/services.dart';
import 'package:flutter/scheduler.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:buzz/shared/theme/buzz_icons.dart';
import 'package:mobile_scanner/mobile_scanner.dart';

import '../../shared/theme/theme.dart';

part 'pairing_qr_scanner/dynamic_island_portal.dart';
part 'pairing_qr_scanner/fallback_scanner.dart';
part 'pairing_qr_scanner/scanner_camera.dart';

const _qrScannerPlatformChannel = MethodChannel('buzz/qr_scanner');

/// The closed Duo camera cutout. Unfolded displays use the drawer scanner.
Rect? duoQrScannerCameraFrame(BuildContext context) {
  if (defaultTargetPlatform != TargetPlatform.iOS) return null;
  final fold = FoldGeometry.of(context);
  if (fold.division != null) return null;

  for (final rect in fold.occlusions) {
    if (rect.width > 0 &&
        (rect.width - rect.height).abs() < 1 &&
        rect.left > 0 &&
        rect.top > 0 &&
        rect.right < fold.size.width &&
        rect.bottom < fold.size.height &&
        (fold.strip?.rect.contains(rect.center) ?? false)) {
      return rect;
    }
  }
  return null;
}

/// Resolves the cover camera even when Flutter publishes no display features.
Future<Rect?> resolveDuoQrScannerCameraFrame(BuildContext context) async {
  if (FoldGeometry.of(context).division != null) return null;
  final reported = duoQrScannerCameraFrame(context);
  if (reported != null || defaultTargetPlatform != TargetPlatform.iOS) {
    return reported;
  }
  final size = MediaQuery.sizeOf(context);
  try {
    final values = await _qrScannerPlatformChannel.invokeListMethod<num>(
      'cameraCutoutFrame',
    );
    if (!context.mounted ||
        MediaQuery.sizeOf(context) != size ||
        values == null ||
        values.length != 6 ||
        values.any((value) => !value.isFinite) ||
        (values[4] - size.width).abs() > 1 ||
        (values[5] - size.height).abs() > 1) {
      return null;
    }
    return Rect.fromLTWH(
      values[0].toDouble(),
      values[1].toDouble(),
      values[2].toDouble(),
      values[3].toDouble(),
    );
  } on PlatformException {
    return null;
  } on MissingPluginException {
    return null;
  }
}

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
Future<String?> showDynamicIslandPairingQrScanner(
  BuildContext context, {
  ValueChanged<double>? onProgress,
  Rect? cameraFrame,
}) {
  return Navigator.of(context).push<String>(
    PageRouteBuilder<String>(
      opaque: false,
      barrierColor: Colors.transparent,
      barrierDismissible: false,
      transitionDuration: Duration.zero,
      reverseTransitionDuration: Duration.zero,
      pageBuilder: (_, _, _) => _DynamicIslandQrScannerPortal(
        onProgress: onProgress,
        cameraFrame: cameraFrame,
        sourceSize: MediaQuery.sizeOf(context),
      ),
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
    this.cameraFrame,
  });

  /// The full logical-pixel size available to the portal route.
  final Size viewport;

  /// The top safe-area inset reported by iOS.
  final double safeAreaTop;

  /// The Duo camera cutout; null retains the Dynamic Island geometry.
  final Rect? cameraFrame;

  static const _collapsedWidth = 120.0;
  static const _collapsedHeight = 36.0;
  static const _fallbackTop = 11.0;
  static const _referenceSafeAreaTop = 59.0;
  static const _edgeMargin = 15.0;
  static const _expandedCornerRadius = 40.0;
  static const _scannerFadeStart = 0.18;

  /// The physical-island-aligned top edge used throughout the morph.
  double get top =>
      _fallbackTop + math.max(0, safeAreaTop - _referenceSafeAreaTop);

  /// The portal frame before the opening animation starts.
  Rect get collapsedFrame {
    final camera = cameraFrame;
    if (camera != null) {
      // Match the cover's active-camera pill seen on-device: four points
      // beyond the cutout at the outer end and 22 points toward the content.
      final onBottom = camera.center.dy > viewport.height / 2;
      return Rect.fromLTRB(
        camera.left,
        camera.top - (onBottom ? 22 : 4),
        camera.right,
        camera.bottom + (onBottom ? 4 : 22),
      );
    }
    return Rect.fromLTWH(
      (viewport.width - _collapsedWidth) / 2,
      top,
      _collapsedWidth,
      _collapsedHeight,
    );
  }

  /// The square camera frame after the opening animation completes.
  Rect get expandedFrame {
    final camera = cameraFrame == null ? null : collapsedFrame;
    if (camera != null) {
      final onRight = camera.center.dx > viewport.width / 2;
      final onBottom = camera.center.dy > viewport.height / 2;
      final horizontalRoom = onRight
          ? camera.right - _edgeMargin
          : viewport.width - _edgeMargin - camera.left;
      final verticalRoom = onBottom
          ? camera.bottom - _edgeMargin
          : viewport.height - _edgeMargin - camera.top;
      final size = math.min(420.0, math.min(horizontalRoom, verticalRoom));
      final left = onRight ? camera.right - size : camera.left;
      final desiredTop = camera.center.dy > viewport.height / 2
          ? camera.bottom - size
          : camera.top;
      final top = desiredTop.clamp(
        _edgeMargin,
        viewport.height - _edgeMargin - size,
      );
      return Rect.fromLTWH(left, top, size, size);
    }
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
      lerpDouble(start.top, end.top, t)!,
      lerpDouble(start.right, end.right, t)!,
      lerpDouble(start.bottom, end.bottom, t)!,
    );
  }

  /// Returns the portal corner radius at [progress].
  double cornerRadiusAt(double progress) {
    final t = progress.clamp(0.0, 1.0);
    return lerpDouble(
      collapsedFrame.shortestSide / 2,
      _expandedCornerRadius,
      t,
    )!;
  }

  /// Keeps the camera-side corner concentric with a circular Duo cutout.
  BorderRadius borderRadiusAt(double progress) {
    final radius = Radius.circular(cornerRadiusAt(progress));
    final camera = cameraFrame;
    if (camera == null || (camera.width - camera.height).abs() >= 1) {
      return BorderRadius.all(radius);
    }
    final anchor = Radius.circular(camera.shortestSide / 2);
    final right = camera.center.dx > viewport.width / 2;
    final bottom = camera.center.dy > viewport.height / 2;
    return BorderRadius.only(
      topLeft: !right && !bottom ? anchor : radius,
      topRight: right && !bottom ? anchor : radius,
      bottomLeft: !right && bottom ? anchor : radius,
      bottomRight: right && bottom ? anchor : radius,
    );
  }

  /// Returns the camera opacity after its delayed entrance.
  double scannerOpacityAt(double progress) {
    final fadeStart = cameraFrame == null ? _scannerFadeStart : 0.35;
    return ((progress - fadeStart) / (1 - fadeStart)).clamp(0.0, 1.0);
  }
}
