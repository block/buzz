import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter/services.dart';
import 'package:flutter_hooks/flutter_hooks.dart';

import '../theme/theme.dart';

/// An iOS-native sheet surface that adopts the system's concentric corners on
/// iOS 26 and newer. Other platforms keep the normal Flutter shape.
class ConcentricSheetSurface extends HookWidget {
  const ConcentricSheetSurface({
    required this.child,
    required this.enabled,
    this.color,
    super.key,
  });

  final Widget child;
  final bool enabled;
  final Color? color;

  static const _surfaceChannel = MethodChannel('buzz/concentric_sheet_surface');

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

    if (!shouldCheckNativeSurface) {
      return child;
    }

    final surfaceColor = color ?? context.colors.surface;

    return Padding(
      padding: const EdgeInsets.only(
        left: Grid.xxs,
        right: Grid.xxs,
        bottom: Grid.xxs,
      ),
      child: Stack(
        children: [
          if (nativeSurfaceSupported)
            Positioned.fill(
              child: ExcludeSemantics(
                child: UiKitView(
                  viewType: 'buzz/concentric_sheet_surface',
                  hitTestBehavior: PlatformViewHitTestBehavior.transparent,
                  creationParams: <String, Object>{
                    'color': surfaceColor.toARGB32(),
                    'minimumRadius': Radii.dialog,
                  },
                  creationParamsCodec: const StandardMessageCodec(),
                ),
              ),
            )
          else
            Positioned.fill(
              child: Material(
                color: surfaceColor,
                borderRadius: BorderRadius.circular(Radii.dialog),
                clipBehavior: Clip.antiAlias,
              ),
            ),
          child,
        ],
      ),
    );
  }
}
