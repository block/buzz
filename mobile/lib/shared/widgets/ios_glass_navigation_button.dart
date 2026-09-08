import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart' show PlatformViewHitTestBehavior;
import 'package:flutter/services.dart';
import 'package:flutter_hooks/flutter_hooks.dart';

import '../theme/theme.dart';

/// The navigation glyph displayed by [IosGlassNavigationButton].
enum IosGlassNavigationIcon {
  back,
  close,
  camera,
  photoLibrary,
  palette,
  droplet,
  emoji,
  person,
  frame,
  rotateCamera,
  shutter,
  colorSwatch,
  sun,
  moon,
  systemAppearance,
}

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
    this.label,
    this.width = 48,
    this.height = 48,
    this.controlSize = 40,
    this.fillWidth = false,
    this.buttonCenterX,
    this.foregroundColor,
    this.swatchColor,
    this.isBusy = false,
    this.isSelected = false,
    this.nativeViewSuppressed,
  });

  static const viewType = 'buzz/navigation_glass';

  /// The SF Symbol-style glyph rendered by the native control.
  final IosGlassNavigationIcon icon;

  /// Optional text rendered by the native control instead of [icon].
  final String? label;

  /// Accessibility label exposed by the native view or Flutter fallback.
  final String semanticLabel;

  /// Invoked when the enabled control is activated.
  final VoidCallback? onPressed;

  /// Width of the platform-view hit target.
  final double width;

  /// Height of the platform-view hit target.
  final double height;

  /// Diameter of the visual glass control inside its hit target.
  final double controlSize;

  /// Whether the native visual control fills the available width.
  final bool fillWidth;

  /// Horizontal center for the visual control within its hit target.
  final double? buttonCenterX;

  /// Optional foreground tint for the native control and Flutter fallback.
  final Color? foregroundColor;

  /// Optional inset color swatch used by [IosGlassNavigationIcon.colorSwatch].
  final Color? swatchColor;

  /// Whether the native control presents its busy state.
  final bool isBusy;

  /// Whether the native control exposes its selected state.
  final bool isSelected;

  /// When true, substitutes an accessible Flutter control for the native view.
  final ValueListenable<bool>? nativeViewSuppressed;

  @override
  Widget build(BuildContext context) {
    assert(defaultTargetPlatform == TargetPlatform.iOS);
    final nativeChannel = useState<MethodChannel?>(null);
    final onPressedRef = useRef(onPressed)..value = onPressed;
    final brightness = context.theme.brightness.name;
    final effectiveForeground = foregroundColor ?? context.colors.primary;
    final foregroundValue = effectiveForeground.toARGB32();
    final swatchColorValue = swatchColor?.toARGB32();
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

    useEffect(
      () {
        final channel = nativeChannel.value;
        if (channel != null) {
          unawaited(
            channel.invokeMethod<void>('setAppearance', <String, Object>{
              'brightness': brightness,
              'foregroundColor': foregroundValue,
              'enabled': enabled,
              'busy': isBusy,
              'selected': isSelected,
              'swatchColor': ?swatchColorValue,
            }),
          );
        }
        return null;
      },
      [
        nativeChannel.value,
        brightness,
        foregroundValue,
        enabled,
        isBusy,
        isSelected,
        swatchColorValue,
      ],
    );

    useEffect(() {
      final channel = nativeChannel.value;
      if (channel != null) {
        final content = <String, Object>{
          'icon': icon.name,
          'accessibilityLabel': semanticLabel,
        };
        if (label != null) content['label'] = label!;
        unawaited(channel.invokeMethod<void>('setContent', content));
      }
      return null;
    }, [nativeChannel.value, icon, label, semanticLabel]);

    Widget buildControl({required bool suppressNativeView}) {
      if (suppressNativeView) {
        final resolvedButtonCenterX = buttonCenterX ?? width / 2;
        return Semantics(
          container: true,
          button: true,
          enabled: enabled,
          selected: isSelected,
          label: semanticLabel,
          onTap: onPressed,
          child: ExcludeSemantics(
            child: Stack(
              key: const ValueKey('ios-glass-navigation-flutter-fallback'),
              children: [
                Positioned(
                  left: resolvedButtonCenterX - controlSize / 2,
                  top: (height - controlSize) / 2,
                  width: controlSize,
                  height: controlSize,
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
                    child: isBusy
                        ? Center(
                            child: SizedBox.square(
                              dimension: 22,
                              child: CircularProgressIndicator(
                                strokeWidth: 2,
                                color: effectiveForeground,
                              ),
                            ),
                          )
                        : icon == IosGlassNavigationIcon.colorSwatch
                        ? Padding(
                            padding: const EdgeInsets.all(4),
                            child: DecoratedBox(
                              decoration: BoxDecoration(
                                color: swatchColor,
                                shape: BoxShape.circle,
                              ),
                            ),
                          )
                        : label != null
                        ? Text(
                            label!,
                            maxLines: 1,
                            style: context.textTheme.labelMedium?.copyWith(
                              color: effectiveForeground,
                              fontWeight: FontWeight.w600,
                            ),
                          )
                        : Icon(
                            switch (icon) {
                              IosGlassNavigationIcon.back =>
                                Icons.arrow_back_ios_new_rounded,
                              IosGlassNavigationIcon.close =>
                                Icons.close_rounded,
                              IosGlassNavigationIcon.camera =>
                                Icons.camera_alt_rounded,
                              IosGlassNavigationIcon.photoLibrary =>
                                Icons.photo_library_rounded,
                              IosGlassNavigationIcon.palette =>
                                Icons.palette_rounded,
                              IosGlassNavigationIcon.droplet =>
                                Icons.water_drop_rounded,
                              IosGlassNavigationIcon.emoji =>
                                Icons.emoji_emotions_rounded,
                              IosGlassNavigationIcon.person =>
                                Icons.person_rounded,
                              IosGlassNavigationIcon.frame =>
                                Icons.photo_size_select_actual_rounded,
                              IosGlassNavigationIcon.rotateCamera =>
                                Icons.cameraswitch_rounded,
                              IosGlassNavigationIcon.shutter => Icons.circle,
                              IosGlassNavigationIcon.colorSwatch =>
                                Icons.circle,
                              IosGlassNavigationIcon.sun =>
                                Icons.light_mode_rounded,
                              IosGlassNavigationIcon.moon =>
                                Icons.dark_mode_rounded,
                              IosGlassNavigationIcon.systemAppearance =>
                                Icons.brightness_auto_rounded,
                            },
                            size: icon == IosGlassNavigationIcon.shutter
                                ? controlSize * 0.72
                                : 22,
                            color: icon == IosGlassNavigationIcon.colorSwatch
                                ? swatchColor
                                : effectiveForeground,
                          ),
                  ),
                ),
              ],
            ),
          ),
        );
      }
      final creationParams = <String, Object>{
        'icon': icon.name,
        'accessibilityLabel': semanticLabel,
        'brightness': brightness,
        'foregroundColor': foregroundValue,
        'enabled': enabled,
        'busy': isBusy,
        'selected': isSelected,
        'controlSize': controlSize,
        'controlWidth': controlSize,
        'fillWidth': fillWidth,
        'buttonCenterX': buttonCenterX ?? width / 2,
        'hitTargetWidth': width,
        'hitTargetHeight': height,
        'swatchColor': ?swatchColorValue,
      };
      if (label != null) creationParams['label'] = label!;
      return UiKitView(
        viewType: viewType,
        hitTestBehavior: PlatformViewHitTestBehavior.opaque,
        creationParams: creationParams,
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
