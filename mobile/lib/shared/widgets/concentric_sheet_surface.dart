import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter/services.dart';

import '../theme/theme.dart';

/// An iOS-native sheet surface that adopts the system's concentric corners on
/// iOS 26 and newer. Other platforms keep the normal Flutter shape.
class ConcentricSheetSurface extends StatefulWidget {
  const ConcentricSheetSurface({
    required this.child,
    required this.enabled,
    this.color,
    super.key,
  });

  final Widget child;
  final bool enabled;
  final Color? color;

  @override
  State<ConcentricSheetSurface> createState() => _ConcentricSheetSurfaceState();
}

class _ConcentricSheetSurfaceState extends State<ConcentricSheetSurface> {
  static const _surfaceChannel = MethodChannel('buzz/concentric_sheet_surface');

  bool _nativeSurfaceSupported = false;

  @override
  void initState() {
    super.initState();
    _checkNativeSurfaceSupport();
  }

  Future<void> _checkNativeSurfaceSupport() async {
    if (!widget.enabled || defaultTargetPlatform != TargetPlatform.iOS) return;

    try {
      final supported = await _surfaceChannel.invokeMethod<bool>('isSupported');
      if (mounted && supported == true) {
        setState(() => _nativeSurfaceSupported = true);
      }
    } on MissingPluginException {
      // The registrar is unavailable, so retain the Flutter surface.
    } on PlatformException {
      // The native surface is optional; retain the Flutter surface on failure.
    }
  }

  @override
  Widget build(BuildContext context) {
    if (!widget.enabled || defaultTargetPlatform != TargetPlatform.iOS) {
      return widget.child;
    }

    final surfaceColor = widget.color ?? context.colors.surface;

    return Padding(
      padding: const EdgeInsets.only(
        left: Grid.xxs,
        right: Grid.xxs,
        bottom: Grid.xxs,
      ),
      child: Stack(
        children: [
          Positioned.fill(
            child: Material(
              color: surfaceColor,
              borderRadius: BorderRadius.circular(Radii.dialog),
              clipBehavior: Clip.antiAlias,
            ),
          ),
          if (_nativeSurfaceSupported)
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
            ),
          widget.child,
        ],
      ),
    );
  }
}
