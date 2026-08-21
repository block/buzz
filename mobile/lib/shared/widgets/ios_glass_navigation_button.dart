import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart' show PlatformViewHitTestBehavior;
import 'package:flutter/services.dart';
import 'package:flutter_hooks/flutter_hooks.dart';

import '../theme/theme.dart';

/// The navigation glyph displayed by [IosGlassNavigationButton].
enum IosGlassNavigationIcon { back, close }

/// Leading width used by iOS channel-style headers.
const iosGlassChannelHeaderLeadingWidth = 58.0;

/// Horizontal center of the native button inside a channel-style leading.
const iosGlassChannelHeaderButtonCenterX = 38.0;

/// Space between the leading region and a channel-style title.
const iosGlassChannelHeaderTitleSpacing = Grid.xs;

/// A native iOS navigation control using the system glass button treatment.
///
/// Callers should only insert this widget on iOS and retain their existing
/// Flutter control on other platforms.
class IosGlassNavigationButton extends HookWidget {
  const IosGlassNavigationButton({
    super.key,
    required this.icon,
    required this.semanticLabel,
    required this.onPressed,
    this.width = 48,
    this.height = 48,
    this.buttonCenterX,
    this.foregroundColor,
    this.nativeViewSuppressed,
  });

  static const viewType = 'buzz/navigation_glass';

  final IosGlassNavigationIcon icon;
  final String semanticLabel;
  final VoidCallback? onPressed;
  final double width;
  final double height;
  final double? buttonCenterX;
  final Color? foregroundColor;
  final ValueListenable<bool>? nativeViewSuppressed;

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
        if (call.method == 'pressed') {
          onPressedRef.value?.call();
        }
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
          }),
        );
      }
      return null;
    }, [nativeChannel.value, brightness, foregroundValue, enabled]);

    Widget buildControl({required bool suppressNativeView}) {
      if (suppressNativeView) {
        final resolvedButtonCenterX = buttonCenterX ?? width / 2;
        return Semantics(
          container: true,
          button: true,
          enabled: enabled,
          label: semanticLabel,
          onTap: onPressed,
          child: ExcludeSemantics(
            child: Stack(
              key: const ValueKey('ios-glass-navigation-flutter-fallback'),
              children: [
                Positioned(
                  left: resolvedButtonCenterX - 20,
                  top: (height - 40) / 2,
                  width: 40,
                  height: 40,
                  child: DecoratedBox(
                    decoration: BoxDecoration(
                      color: context.colors.surface.withValues(alpha: 0.72),
                      shape: BoxShape.circle,
                      border: Border.all(
                        color: context.colors.inverseSurface.withValues(
                          alpha: 0.07,
                        ),
                      ),
                    ),
                    child: Icon(
                      icon == IosGlassNavigationIcon.back
                          ? Icons.arrow_back_ios_new_rounded
                          : Icons.close_rounded,
                      size: 20,
                      color: effectiveForeground,
                    ),
                  ),
                ),
              ],
            ),
          ),
        );
      }
      return UiKitView(
        viewType: viewType,
        hitTestBehavior: PlatformViewHitTestBehavior.opaque,
        creationParams: <String, Object>{
          'icon': icon.name,
          'accessibilityLabel': semanticLabel,
          'brightness': brightness,
          'foregroundColor': foregroundValue,
          'enabled': enabled,
          'buttonCenterX': buttonCenterX ?? width / 2,
          'hitTargetWidth': width,
          'hitTargetHeight': height,
        },
        creationParamsCodec: const StandardMessageCodec(),
        onPlatformViewCreated: (viewId) {
          nativeChannel.value = MethodChannel('$viewType/$viewId');
        },
      );
    }

    return Tooltip(
      message: semanticLabel,
      excludeFromSemantics: true,
      child: SizedBox(
        width: width,
        height: height,
        child: nativeViewSuppressed == null
            ? buildControl(suppressNativeView: false)
            : ValueListenableBuilder<bool>(
                valueListenable: nativeViewSuppressed!,
                builder: (context, suppressNativeView, _) =>
                    buildControl(suppressNativeView: suppressNativeView),
              ),
      ),
    );
  }
}
