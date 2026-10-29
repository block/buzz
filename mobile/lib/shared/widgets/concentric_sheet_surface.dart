import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter/services.dart';
import 'package:flutter_hooks/flutter_hooks.dart';

import '../theme/theme.dart';

enum ConcentricSurfaceCorners { all, bottom }

/// An iOS-native surface that adopts the system's concentric corners on iOS 26
/// and newer. Other platforms keep the normal Flutter shape.
class ConcentricSheetSurface extends HookWidget {
  const ConcentricSheetSurface({
    required this.child,
    required this.enabled,
    this.color,
    this.backdropColor,
    this.corners = ConcentricSurfaceCorners.all,
    this.padding = const EdgeInsets.only(
      left: Grid.xxs,
      right: Grid.xxs,
      bottom: Grid.xxs,
    ),
    this.providesSheetSurface = true,
    this.usesGlass = false,
    this.glassTintColor,
    this.minimumRadius = Radii.dialog,
    this.contentClipRadius,
    super.key,
  });

  final Widget child;
  final bool enabled;
  final Color? color;
  final Color? backdropColor;
  final ConcentricSurfaceCorners corners;
  final EdgeInsetsGeometry padding;
  final bool providesSheetSurface;
  final bool usesGlass;

  /// Optional tint applied to the native Liquid Glass material.
  final Color? glassTintColor;
  final double minimumRadius;
  final double? contentClipRadius;

  static bool providesSurfaceOf(BuildContext context) =>
      context
          .dependOnInheritedWidgetOfExactType<_ConcentricSheetSurfaceScope>()
          ?.providesSurface ??
      false;

  static const _surfaceChannel = MethodChannel('buzz/concentric_sheet_surface');
  static const _nativeContentClipRadius = Radii.dialog * 2;
  static const sheetTopRadius = _nativeContentClipRadius - Grid.xxs;

  BorderRadius _borderRadius(double radius) => switch (corners) {
    ConcentricSurfaceCorners.all => BorderRadius.circular(radius),
    ConcentricSurfaceCorners.bottom => BorderRadius.vertical(
      bottom: Radius.circular(radius),
    ),
  };

  Future<bool> _checkNativeSurfaceSupport() async {
    try {
      final supported = await _surfaceChannel.invokeMethod<bool>('isSupported');
      return supported == true;
    } on MissingPluginException {
      // The registrar is unavailable, so retain the Flutter surface.
      return false;
    } on PlatformException {
      // The native surface is optional; retain the Flutter surface on failure.
      return false;
    }
  }

  Future<void> _updateNativeSurfaceColors({
    required MethodChannel channel,
    required Color surfaceColor,
    required Color? backdropColor,
  }) async {
    try {
      await channel.invokeMethod<void>('updateColors', <String, Object?>{
        'color': surfaceColor.toARGB32(),
        'backdropColor': backdropColor?.toARGB32(),
        'glassTintColor': glassTintColor?.toARGB32(),
      });
    } on MissingPluginException {
      // The platform view may have been disposed while its theme was changing.
    } on PlatformException {
      // The native surface is optional; retain its last successfully sent color.
    }
  }

  Future<void> _updateNativeSurfaceGeometry({
    required MethodChannel channel,
    required double minimumRadius,
    required Brightness brightness,
  }) async {
    try {
      await channel.invokeMethod<void>('updateGeometry', <String, Object>{
        'minimumRadius': minimumRadius,
        'brightness': brightness.name,
      });
    } on MissingPluginException {
      // The platform view may have been disposed while its shape was changing.
    } on PlatformException {
      // The native surface is optional; retain its last successful geometry.
    }
  }

  @override
  Widget build(BuildContext context) {
    final shouldCheckNativeSurface =
        enabled && defaultTargetPlatform == TargetPlatform.iOS;
    final supportFuture = useMemoized(
      () => shouldCheckNativeSurface
          ? _checkNativeSurfaceSupport()
          : Future<bool>.value(false),
      [shouldCheckNativeSurface],
    );
    final nativeSurfaceSupported = useFuture(supportFuture).data ?? false;
    final surfaceColor = color ?? context.colors.surface;
    final brightness = context.theme.brightness;
    final nativeSurfaceChannel = useState<MethodChannel?>(null);
    final nativeCornerRadius = useState<BorderRadius?>(null);
    useEffect(() {
      final channel = nativeSurfaceChannel.value;
      if (channel == null) return null;
      var active = true;
      void updateRadii(dynamic value) {
        if (!active || value is! Map) return;
        double? radius(String key) {
          final number = value[key];
          return number is num && number.isFinite && number >= 0
              ? number.toDouble()
              : null;
        }

        final topLeft = radius('topLeft');
        final topRight = radius('topRight');
        final bottomLeft = radius('bottomLeft');
        final bottomRight = radius('bottomRight');
        if (topLeft == null ||
            topRight == null ||
            bottomLeft == null ||
            bottomRight == null) {
          return;
        }
        nativeCornerRadius.value = BorderRadius.only(
          topLeft: Radius.circular(topLeft),
          topRight: Radius.circular(topRight),
          bottomLeft: Radius.circular(bottomLeft),
          bottomRight: Radius.circular(bottomRight),
        );
      }

      channel.setMethodCallHandler((call) async {
        if (call.method == 'cornerRadiiChanged') updateRadii(call.arguments);
      });
      // The platform view can lay out before Flutter installs its handler.
      Future<void> readInitialRadii() async {
        try {
          updateRadii(await channel.invokeMethod<dynamic>('getCornerRadii'));
        } on MissingPluginException {
          // Retain the fallback clip when the native view is unavailable.
        } on PlatformException {
          // A subsequent native layout can supply the geometry.
        }
      }

      unawaited(readInitialRadii());
      return () {
        active = false;
        channel.setMethodCallHandler(null);
      };
    }, [nativeSurfaceChannel.value]);
    useEffect(
      () {
        final channel = nativeSurfaceChannel.value;
        if (!shouldCheckNativeSurface ||
            !nativeSurfaceSupported ||
            channel == null) {
          return null;
        }
        unawaited(
          _updateNativeSurfaceColors(
            channel: channel,
            surfaceColor: surfaceColor,
            backdropColor: backdropColor,
          ),
        );
        return null;
      },
      [
        shouldCheckNativeSurface,
        nativeSurfaceSupported,
        nativeSurfaceChannel.value,
        surfaceColor,
        backdropColor,
        glassTintColor,
      ],
    );
    useEffect(
      () {
        final channel = nativeSurfaceChannel.value;
        if (!shouldCheckNativeSurface ||
            !nativeSurfaceSupported ||
            channel == null) {
          return null;
        }
        unawaited(
          _updateNativeSurfaceGeometry(
            channel: channel,
            minimumRadius: minimumRadius,
            brightness: brightness,
          ),
        );
        return null;
      },
      [
        shouldCheckNativeSurface,
        nativeSurfaceSupported,
        nativeSurfaceChannel.value,
        minimumRadius,
        brightness,
      ],
    );

    if (!shouldCheckNativeSurface) {
      return _ConcentricSheetSurfaceScope(providesSurface: false, child: child);
    }

    final fallbackBorderRadius = _borderRadius(minimumRadius);

    return Padding(
      padding: padding,
      child: Stack(
        children: [
          if (nativeSurfaceSupported)
            Positioned.fill(
              child: ExcludeSemantics(
                child: UiKitView(
                  viewType: 'buzz/concentric_sheet_surface',
                  // Keep the platform view in gesture arbitration. Excluding
                  // it with IgnorePointer leaves UIKit intercepting touches
                  // over Flutter controls after the native surface appears.
                  hitTestBehavior: PlatformViewHitTestBehavior.translucent,
                  onPlatformViewCreated: (viewId) {
                    nativeSurfaceChannel.value = MethodChannel(
                      'buzz/concentric_sheet_surface/$viewId',
                    );
                  },
                  creationParams: <String, Object>{
                    'color': surfaceColor.toARGB32(),
                    if (backdropColor case final color?)
                      'backdropColor': color.toARGB32(),
                    'minimumRadius': minimumRadius,
                    'corners': corners.name,
                    'independentCorners': providesSheetSurface,
                    if (providesSheetSurface &&
                        corners == ConcentricSurfaceCorners.all)
                      'topRadius': sheetTopRadius,
                    'usesGlass': usesGlass,
                    if (glassTintColor case final tint?)
                      'glassTintColor': tint.toARGB32(),
                    'brightness': brightness.name,
                  },
                  creationParamsCodec: const StandardMessageCodec(),
                ),
              ),
            )
          else
            Positioned.fill(
              child: Material(
                color: surfaceColor,
                borderRadius: fallbackBorderRadius,
                clipBehavior: Clip.antiAlias,
              ),
            ),
          ClipRSuperellipse(
            key: const ValueKey('concentric-sheet-content-clip'),
            // Match each resolved UIKit corner, including asymmetric display
            // corners and changes when folding or rotating the device.
            borderRadius: contentClipRadius != null
                ? _borderRadius(contentClipRadius!)
                : nativeSurfaceSupported
                ? (nativeCornerRadius.value ??
                      (providesSheetSurface &&
                              corners == ConcentricSurfaceCorners.all
                          ? const BorderRadius.vertical(
                              top: Radius.circular(sheetTopRadius),
                              bottom: Radius.circular(_nativeContentClipRadius),
                            )
                          : _borderRadius(_nativeContentClipRadius)))
                : fallbackBorderRadius,
            clipBehavior: Clip.antiAlias,
            child: providesSheetSurface
                ? _ConcentricSheetSurfaceScope(
                    providesSurface: true,
                    child: child,
                  )
                : child,
          ),
        ],
      ),
    );
  }
}

class _ConcentricSheetSurfaceScope extends InheritedWidget {
  const _ConcentricSheetSurfaceScope({
    required this.providesSurface,
    required super.child,
  });

  final bool providesSurface;

  @override
  bool updateShouldNotify(_ConcentricSheetSurfaceScope oldWidget) =>
      oldWidget.providesSurface != providesSurface;
}
