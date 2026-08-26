import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart' show PlatformViewHitTestBehavior;
import 'package:flutter/services.dart';
import 'package:flutter_hooks/flutter_hooks.dart';

import '../theme/theme.dart';
import 'ios_glass_navigation_button.dart';

/// A native iOS glass capsule for a short navigation-bar text action.
class IosGlassNavigationAction extends HookWidget {
  /// Creates a native glass navigation action.
  const IosGlassNavigationAction({
    super.key,
    required this.label,
    required this.onPressed,
    this.width = 64,
    this.height = 48,
    this.foregroundColor,
    this.isBusy = false,
  });

  /// The visible and accessible action label.
  final String label;

  /// Called when the action is pressed, or null when disabled.
  final VoidCallback? onPressed;

  /// The width of the Flutter platform-view region.
  final double width;

  /// The height of the Flutter platform-view region.
  final double height;

  /// Optional foreground tint; defaults to the active theme color.
  final Color? foregroundColor;

  /// Whether the native action should replace its label with progress.
  final bool isBusy;

  @override
  Widget build(BuildContext context) {
    assert(defaultTargetPlatform == TargetPlatform.iOS);
    final nativeChannel = useState<MethodChannel?>(null);
    final onPressedRef = useRef(onPressed)..value = onPressed;
    final brightness = context.theme.brightness.name;
    final effectiveForeground = foregroundColor ?? context.colors.primary;
    final foregroundValue = effectiveForeground.toARGB32();
    final enabled = onPressed != null;

    useEffect(() {
      final channel = nativeChannel.value;
      if (channel == null) return null;
      channel.setMethodCallHandler((call) async {
        if (call.method == 'pressed') onPressedRef.value?.call();
      });
      return () => channel.setMethodCallHandler(null);
    }, [nativeChannel.value]);

    useEffect(() {
      final channel = nativeChannel.value;
      if (channel != null) {
        unawaited(
          channel.invokeMethod<void>('setAppearance', <String, Object>{
            'brightness': brightness,
            'foregroundColor': foregroundValue,
            'enabled': enabled,
            'busy': isBusy,
          }),
        );
      }
      return null;
    }, [nativeChannel.value, brightness, foregroundValue, enabled, isBusy]);

    return Tooltip(
      message: label,
      excludeFromSemantics: true,
      child: SizedBox(
        width: width,
        height: height,
        child: UiKitView(
          viewType: IosGlassNavigationButton.viewType,
          hitTestBehavior: PlatformViewHitTestBehavior.opaque,
          creationParams: <String, Object>{
            'label': label,
            'accessibilityLabel': label,
            'brightness': brightness,
            'foregroundColor': foregroundValue,
            'enabled': enabled,
            'busy': isBusy,
            'buttonCenterX': width / 2,
            'controlWidth': width - 8,
            'hitTargetWidth': width,
            'hitTargetHeight': height,
          },
          creationParamsCodec: const StandardMessageCodec(),
          onPlatformViewCreated: (viewId) {
            nativeChannel.value = MethodChannel(
              '${IosGlassNavigationButton.viewType}/$viewId',
            );
          },
        ),
      ),
    );
  }
}
