import 'dart:async';
import 'dart:ui' show ImageFilter;

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart' show PlatformViewHitTestBehavior;
import 'package:flutter/services.dart';
import 'package:flutter_hooks/flutter_hooks.dart';

import '../theme/theme.dart';
import 'avatar_image.dart';
import 'buzz_navigation_metrics.dart';

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
  avatar,
  channel,
  headphones,
  users,
  more,
}

/// A native contextual-menu item attached to an iOS glass navigation button.
class IosGlassNavigationMenuItem {
  const IosGlassNavigationMenuItem({
    required this.id,
    required this.label,
    this.selected = false,
    this.destructive = false,
    this.avatarImageUrl,
    this.avatarFallback,
    this.systemIconName,
    this.keepsSingleLine = false,
  });

  final String id;
  final String label;
  final bool selected;
  final bool destructive;
  final String? avatarImageUrl;
  final String? avatarFallback;
  final String? systemIconName;
  final bool keepsSingleLine;

  Map<String, Object> toJson() {
    final json = <String, Object>{
      'id': id,
      'label': label,
      'selected': selected,
      'destructive': destructive,
      'keepsSingleLine': keepsSingleLine,
    };
    if (avatarImageUrl != null) json['avatarImageUrl'] = avatarImageUrl!;
    if (avatarFallback != null) json['avatarFallback'] = avatarFallback!;
    if (systemIconName != null) json['systemIconName'] = systemIconName!;
    return json;
  }
}

/// Leading width used by iOS channel-style headers.
const iosGlassChannelHeaderLeadingWidth = 48.0;

/// Horizontal center of the native button inside a channel-style leading.
const iosGlassChannelHeaderButtonCenterX =
    iosGlassChannelHeaderLeadingWidth / 2;

/// Insets channel-style controls so their glass edges match chat content.
const iosGlassChannelHeaderHorizontalInset =
    Grid.gutter -
    (iosGlassChannelHeaderLeadingWidth - buzzNavigationActionSize) / 2;

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
    this.subtitle,
    this.width = 48,
    this.height = 48,
    this.controlSize = buzzNavigationActionSize,
    this.fillWidth = false,
    this.buttonCenterX,
    this.foregroundColor,
    this.selectionColor,
    this.swatchColor,
    this.isBusy = false,
    this.isSelected = false,
    this.nativeViewSuppressed,
    this.avatarImageUrl,
    this.avatarFallback,
    this.systemIconName,
    this.menuItems = const [],
    this.onMenuSelected,
  });

  static const viewType = 'buzz/navigation_glass';

  /// The SF Symbol-style glyph rendered by the native control.
  final IosGlassNavigationIcon icon;

  /// Optional text rendered by the native control instead of [icon].
  final String? label;

  /// Optional secondary text rendered beneath [label].
  final String? subtitle;

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

  /// Optional fill tint used while the control is selected.
  final Color? selectionColor;

  /// Optional inset color swatch used by [IosGlassNavigationIcon.colorSwatch].
  final Color? swatchColor;

  /// Whether the native control presents its busy state.
  final bool isBusy;

  /// Whether the native control exposes its selected state.
  final bool isSelected;

  /// When true, substitutes an accessible Flutter control for the native view.
  final ValueListenable<bool>? nativeViewSuppressed;

  /// Optional avatar content used with [IosGlassNavigationIcon.avatar].
  final String? avatarImageUrl;

  /// Initial shown while an avatar image is unavailable.
  final String? avatarFallback;

  /// Optional SF Symbol used by native content such as channel identities.
  final String? systemIconName;

  /// Native menu presented directly beneath the glass control.
  final List<IosGlassNavigationMenuItem> menuItems;

  /// Invoked when a native menu item is selected.
  final ValueChanged<String>? onMenuSelected;

  @override
  Widget build(BuildContext context) {
    assert(defaultTargetPlatform == TargetPlatform.iOS);
    final nativeChannel = useState<MethodChannel?>(null);
    final onPressedRef = useRef(onPressed)..value = onPressed;
    final onMenuSelectedRef = useRef(onMenuSelected)..value = onMenuSelected;
    final brightness = context.theme.brightness.name;
    final effectiveForeground = foregroundColor ?? context.colors.primary;
    final effectiveSelection = selectionColor ?? effectiveForeground;
    final foregroundValue = effectiveForeground.toARGB32();
    final selectionValue = effectiveSelection.toARGB32();
    final swatchColorValue = swatchColor?.toARGB32();
    final enabled = onPressed != null;
    final routeAnimation =
        ModalRoute.of(context)?.animation ??
        const AlwaysStoppedAnimation<double>(1);
    final routeIsTransitioning = useState(
      routeAnimation.status == AnimationStatus.forward,
    );
    useEffect(() {
      void updateRouteTransition(AnimationStatus status) {
        // Keep the real UIKit control during an interactive pop. Replacing it
        // while the user drags backward exposes the Flutter approximation.
        final next = status == AnimationStatus.forward;
        if (routeIsTransitioning.value != next) {
          routeIsTransitioning.value = next;
        }
      }

      routeAnimation.addStatusListener(updateRouteTransition);
      return () => routeAnimation.removeStatusListener(updateRouteTransition);
    }, [routeAnimation]);
    final menuSignature = menuItems
        .map(
          (item) =>
              '${item.id}:${item.label}:${item.selected}:${item.destructive}:'
              '${item.avatarImageUrl}:${item.avatarFallback}:'
              '${item.systemIconName}:${item.keepsSingleLine}',
        )
        .join('|');

    useEffect(() {
      final channel = nativeChannel.value;
      if (channel == null) return null;
      channel.setMethodCallHandler((call) async {
        if (call.method == 'pressed') {
          onPressedRef.value?.call();
        } else if (call.method == 'selected' && call.arguments is String) {
          onMenuSelectedRef.value?.call(call.arguments as String);
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
              'selectionColor': selectionValue,
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
        selectionValue,
        enabled,
        isBusy,
        isSelected,
        swatchColorValue,
      ],
    );

    useEffect(
      () {
        final channel = nativeChannel.value;
        if (channel != null) {
          final content = <String, Object>{
            'icon': icon.name,
            'accessibilityLabel': semanticLabel,
          };
          if (label != null) content['label'] = label!;
          if (subtitle != null) content['subtitle'] = subtitle!;
          if (avatarImageUrl != null) {
            content['avatarImageUrl'] = avatarImageUrl!;
          }
          if (avatarFallback != null) {
            content['avatarFallback'] = avatarFallback!;
          }
          if (systemIconName != null) {
            content['systemIconName'] = systemIconName!;
          }
          content['menuItems'] = menuItems
              .map((item) => item.toJson())
              .toList();
          unawaited(channel.invokeMethod<void>('setContent', content));
        }
        return null;
      },
      [
        nativeChannel.value,
        icon,
        label,
        subtitle,
        semanticLabel,
        avatarImageUrl,
        avatarFallback,
        systemIconName,
        menuSignature,
      ],
    );

    Widget buildControl({required bool suppressNativeView}) {
      if (suppressNativeView) {
        final resolvedButtonCenterX = buttonCenterX ?? width / 2;
        final isLeadingContent =
            icon == IosGlassNavigationIcon.avatar ||
            icon == IosGlassNavigationIcon.channel;
        final isAvatarContent = icon == IosGlassNavigationIcon.avatar;
        final fallbackIcon = switch (icon) {
          IosGlassNavigationIcon.back => Icons.arrow_back_ios_new_rounded,
          IosGlassNavigationIcon.close => Icons.close_rounded,
          IosGlassNavigationIcon.camera => Icons.camera_alt_rounded,
          IosGlassNavigationIcon.photoLibrary => Icons.photo_library_rounded,
          IosGlassNavigationIcon.palette => Icons.palette_rounded,
          IosGlassNavigationIcon.droplet => Icons.water_drop_rounded,
          IosGlassNavigationIcon.emoji => Icons.emoji_emotions_rounded,
          IosGlassNavigationIcon.person => Icons.person_rounded,
          IosGlassNavigationIcon.frame =>
            Icons.photo_size_select_actual_rounded,
          IosGlassNavigationIcon.rotateCamera => Icons.cameraswitch_rounded,
          IosGlassNavigationIcon.shutter => Icons.circle,
          IosGlassNavigationIcon.colorSwatch => Icons.circle,
          IosGlassNavigationIcon.sun => Icons.light_mode_rounded,
          IosGlassNavigationIcon.moon => Icons.dark_mode_rounded,
          IosGlassNavigationIcon.systemAppearance =>
            Icons.brightness_auto_rounded,
          IosGlassNavigationIcon.avatar => Icons.person,
          IosGlassNavigationIcon.channel => switch (systemIconName) {
            'lock.fill' => Icons.lock_rounded,
            'bubble.left.and.bubble.right.fill' => Icons.forum_rounded,
            _ => Icons.tag_rounded,
          },
          IosGlassNavigationIcon.headphones => Icons.headphones_rounded,
          IosGlassNavigationIcon.users => Icons.group_rounded,
          IosGlassNavigationIcon.more => Icons.more_vert_rounded,
        };
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
                  left: fillWidth ? 0 : resolvedButtonCenterX - controlSize / 2,
                  right: fillWidth ? 0 : null,
                  top: (height - controlSize) / 2,
                  width: fillWidth ? null : controlSize,
                  height: controlSize,
                  child: ClipRRect(
                    borderRadius: BorderRadius.circular(controlSize / 2),
                    child: BackdropFilter(
                      filter: ImageFilter.blur(sigmaX: 18, sigmaY: 18),
                      child: DecoratedBox(
                        decoration: BoxDecoration(
                          color: isSelected
                              ? effectiveSelection
                              : context.colors.surface.withValues(alpha: 0.68),
                          borderRadius: BorderRadius.circular(controlSize / 2),
                          border: Border.all(
                            color: context.colors.inverseSurface.withValues(
                              alpha: 0.08,
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
                            : isLeadingContent &&
                                  (label != null ||
                                      icon == IosGlassNavigationIcon.avatar)
                            ? Padding(
                                padding: EdgeInsetsDirectional.only(
                                  start: isAvatarContent ? 6 : 12,
                                  end: isAvatarContent
                                      ? (label == null ? 6 : 8)
                                      : 16,
                                ),
                                child: Row(
                                  key: const ValueKey(
                                    'ios-glass-navigation-leading-content',
                                  ),
                                  children: [
                                    if (isAvatarContent)
                                      AvatarImage(
                                        key: const ValueKey(
                                          'ios-glass-navigation-leading-image',
                                        ),
                                        imageUrl: avatarImageUrl,
                                        radius: (controlSize - 12) / 2,
                                        backgroundColor:
                                            context.colors.primaryContainer,
                                        fallback: Text(
                                          avatarFallback ?? '?',
                                          style: context.textTheme.labelMedium
                                              ?.copyWith(
                                                color: context
                                                    .colors
                                                    .onPrimaryContainer,
                                                fontWeight: FontWeight.w600,
                                              ),
                                        ),
                                      )
                                    else
                                      SizedBox.square(
                                        key: const ValueKey(
                                          'ios-glass-navigation-leading-image',
                                        ),
                                        dimension: 12,
                                        child: Icon(
                                          fallbackIcon,
                                          size: 12,
                                          color: effectiveForeground,
                                        ),
                                      ),
                                    if (label != null) ...[
                                      const SizedBox(width: Grid.xxs),
                                      Expanded(
                                        child: Column(
                                          key: const ValueKey(
                                            'ios-glass-navigation-leading-text',
                                          ),
                                          mainAxisAlignment:
                                              MainAxisAlignment.center,
                                          crossAxisAlignment:
                                              CrossAxisAlignment.start,
                                          children: [
                                            Text(
                                              label!,
                                              maxLines: 1,
                                              overflow: TextOverflow.ellipsis,
                                              textScaler: TextScaler.noScaling,
                                              style:
                                                  (icon ==
                                                              IosGlassNavigationIcon
                                                                  .channel
                                                          ? context
                                                                .textTheme
                                                                .titleSmall
                                                          : context
                                                                .textTheme
                                                                .titleMedium)
                                                      ?.copyWith(
                                                        color:
                                                            effectiveForeground,
                                                        fontWeight:
                                                            FontWeight.w600,
                                                      ),
                                            ),
                                            if (subtitle != null)
                                              Text(
                                                subtitle!,
                                                maxLines: 1,
                                                overflow: TextOverflow.ellipsis,
                                                textScaler:
                                                    TextScaler.noScaling,
                                                style: context
                                                    .textTheme
                                                    .bodySmall
                                                    ?.copyWith(
                                                      color: context
                                                          .colors
                                                          .onSurface
                                                          .withValues(
                                                            alpha: 0.65,
                                                          ),
                                                    ),
                                              ),
                                          ],
                                        ),
                                      ),
                                    ],
                                  ],
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
                                fallbackIcon,
                                size: icon == IosGlassNavigationIcon.shutter
                                    ? controlSize * 0.72
                                    : 17,
                                color:
                                    icon == IosGlassNavigationIcon.colorSwatch
                                    ? swatchColor
                                    : effectiveForeground,
                              ),
                      ),
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
        'selectionColor': selectionValue,
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
        'avatarImageUrl': ?avatarImageUrl,
        'avatarFallback': ?avatarFallback,
        'subtitle': ?subtitle,
        'systemIconName': ?systemIconName,
        'menuItems': menuItems.map((item) => item.toJson()).toList(),
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

    Widget buildLayeredControl({required bool suppressNativeView}) {
      final reduceMotion = MediaQuery.disableAnimationsOf(context);
      return Stack(
        fit: StackFit.expand,
        children: [
          IgnorePointer(
            ignoring: suppressNativeView,
            child: ExcludeSemantics(
              excluding: suppressNativeView,
              child: Opacity(
                key: const ValueKey('ios-glass-navigation-native-layer'),
                opacity: suppressNativeView ? 0 : 1,
                child: buildControl(suppressNativeView: false),
              ),
            ),
          ),
          IgnorePointer(
            ignoring: !suppressNativeView,
            child: ExcludeSemantics(
              excluding: !suppressNativeView,
              child: AnimatedOpacity(
                key: const ValueKey('ios-glass-navigation-fallback-layer'),
                opacity: suppressNativeView ? 1 : 0,
                duration: reduceMotion
                    ? Duration.zero
                    : const Duration(milliseconds: 120),
                curve: Curves.easeOutCubic,
                child: buildControl(suppressNativeView: true),
              ),
            ),
          ),
        ],
      );
    }

    return Tooltip(
      message: semanticLabel,
      excludeFromSemantics: true,
      child: SizedBox(
        width: width,
        height: height,
        child: nativeViewSuppressed == null
            ? buildLayeredControl(
                suppressNativeView: routeIsTransitioning.value,
              )
            : ValueListenableBuilder<bool>(
                valueListenable: nativeViewSuppressed!,
                builder: (context, suppressNativeView, _) => suppressNativeView
                    ? buildControl(suppressNativeView: true)
                    : buildLayeredControl(
                        suppressNativeView: routeIsTransitioning.value,
                      ),
              ),
      ),
    );
  }
}
