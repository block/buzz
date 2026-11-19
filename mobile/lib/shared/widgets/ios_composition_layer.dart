import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart' show PlatformViewHitTestBehavior;
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:flutter/services.dart';

/// Starts an unclipped Flutter composition slice above preceding native views.
///
/// iOS applies a platform view's rounded clip to the Flutter overlay slice
/// painted after it. Start an unclipped native slice so a sheet's close button
/// cannot clip later sheet or toast content beneath earlier native views.
class IosCompositionLayer extends HookWidget {
  const IosCompositionLayer({super.key, required this.child, this.color});

  final Widget child;

  /// An opaque native backing for a moving full-screen sheet.
  final Color? color;

  @override
  Widget build(BuildContext context) {
    final ready = useState(false);
    if (defaultTargetPlatform != TargetPlatform.iOS) return child;
    final surface = ExcludeSemantics(
      child: ExcludeFocus(
        child: UiKitView(
          key: ValueKey(color),
          viewType: 'buzz/toast_layer',
          hitTestBehavior: PlatformViewHitTestBehavior.transparent,
          creationParams: {'color': color?.toARGB32()},
          creationParamsCodec: const StandardMessageCodec(),
          onPlatformViewCreated: (_) => ready.value = true,
        ),
      ),
    );
    return Stack(
      fit: StackFit.expand,
      children: [
        if (color != null)
          Positioned.fill(child: surface)
        else
          Positioned(top: 0, left: 0, width: 1, height: 1, child: surface),
        if (color != null || ready.value) child,
      ],
    );
  }
}
