import 'dart:async';
import 'dart:math';
import 'dart:ui';

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter/services.dart';
import 'package:flutter_hooks/flutter_hooks.dart';

import '../../shared/theme/theme.dart';
import 'message_action_backdrop_state.dart';

/// The active date and vertical push-off applied to a sticky date header.
@immutable
class StickyDateHeaderState {
  final String? label;
  final double translateY;

  const StickyDateHeaderState({this.label, this.translateY = 0});

  static const hidden = StickyDateHeaderState();

  bool get isVisible => label != null;

  @override
  bool operator ==(Object other) {
    return other is StickyDateHeaderState &&
        other.label == label &&
        other.translateY == translateY;
  }

  @override
  int get hashCode => Object.hash(label, translateY);
}

/// A glass date capsule that remains below the app bar as its day scrolls.
class StickyDateHeader extends StatelessWidget {
  final ValueListenable<StickyDateHeaderState> state;

  const StickyDateHeader({required this.state, super.key});

  static const _iosViewType = 'buzz/sticky_date_glass';
  static const _minimumIosGlassHeight = 28.0;

  /// Height used by the surface and the next-day push-off calculation.
  static double heightOf(BuildContext context) {
    final labelStyle = context.textTheme.labelMedium;
    final unscaledLineHeight =
        (labelStyle?.fontSize ?? 14) * (labelStyle?.height ?? 1.25);
    final contentHeight =
        MediaQuery.textScalerOf(context).scale(unscaledLineHeight) + Grid.xxs;
    return defaultTargetPlatform == TargetPlatform.iOS
        ? max(_minimumIosGlassHeight, contentHeight)
        : contentHeight;
  }

  Widget _buildIosGlass(BuildContext context, String label) {
    final textStyle = context.textTheme.labelMedium?.copyWith(
      color: context.colors.onSurfaceVariant,
      fontWeight: FontWeight.w500,
    );
    final textPainter = TextPainter(
      text: TextSpan(text: label, style: textStyle),
      maxLines: 1,
      textDirection: Directionality.of(context),
      textScaler: MediaQuery.textScalerOf(context),
    )..layout();

    return _IosStickyDateGlass(
      label: label,
      width: textPainter.width + Grid.sm,
      height: heightOf(context),
    );
  }

  Widget _buildFlutterSurface(BuildContext context, String label) {
    return Container(
      decoration: BoxDecoration(
        borderRadius: BorderRadius.circular(Radii.full),
        boxShadow: [
          BoxShadow(
            color: Colors.black.withValues(alpha: 0.12),
            blurRadius: 8,
            offset: const Offset(0, 2),
          ),
        ],
      ),
      child: ClipRRect(
        borderRadius: BorderRadius.circular(Radii.full),
        child: BackdropFilter(
          filter: ImageFilter.blur(sigmaX: 20, sigmaY: 20),
          child: ConstrainedBox(
            key: const ValueKey('channel-sticky-date-header-surface'),
            constraints: BoxConstraints(minHeight: heightOf(context)),
            child: DecoratedBox(
              decoration: BoxDecoration(
                color: context.colors.surface.withValues(alpha: 0.82),
                borderRadius: BorderRadius.circular(Radii.full),
                border: Border.all(
                  color: context.colors.onSurface.withValues(alpha: 0.08),
                ),
              ),
              child: Padding(
                padding: const EdgeInsets.symmetric(
                  horizontal: Grid.twelve,
                  vertical: Grid.half,
                ),
                child: Semantics(
                  header: true,
                  child: Text(
                    label,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: context.textTheme.labelMedium?.copyWith(
                      color: context.colors.onSurfaceVariant,
                      fontWeight: FontWeight.w500,
                    ),
                  ),
                ),
              ),
            ),
          ),
        ),
      ),
    );
  }

  @override
  Widget build(BuildContext context) {
    final reducedMotion = MediaQuery.disableAnimationsOf(context);

    return ValueListenableBuilder<StickyDateHeaderState>(
      valueListenable: state,
      builder: (context, value, _) {
        final stickyHeight = heightOf(context);
        return SizedBox(
          height: stickyHeight,
          child: IgnorePointer(
            child: ExcludeSemantics(
              excluding: !value.isVisible,
              child: AnimatedOpacity(
                duration: reducedMotion
                    ? Duration.zero
                    : const Duration(milliseconds: 120),
                curve: Curves.easeOutCubic,
                opacity: value.isVisible ? 1 : 0,
                child: ValueListenableBuilder<bool>(
                  valueListenable: messageActionBackdropActive,
                  builder: (context, backdropActive, _) {
                    final useNativeGlass =
                        defaultTargetPlatform == TargetPlatform.iOS &&
                        !backdropActive;
                    final surface = Center(
                      child: RepaintBoundary(
                        child: useNativeGlass
                            ? _buildIosGlass(context, value.label ?? '')
                            : _buildFlutterSurface(context, value.label ?? ''),
                      ),
                    );

                    final pushOffProgress =
                        (-value.translateY / (stickyHeight + 5)).clamp(
                          0.0,
                          1.0,
                        );
                    return Opacity(
                      key: const ValueKey('sticky-date-push-off-opacity'),
                      opacity: 1 - pushOffProgress,
                      child: surface,
                    );
                  },
                ),
              ),
            ),
          ),
        );
      },
    );
  }
}

class _IosStickyDateGlass extends HookWidget {
  final String label;
  final double width;
  final double height;

  const _IosStickyDateGlass({
    required this.label,
    required this.width,
    required this.height,
  });

  @override
  Widget build(BuildContext context) {
    final nativeChannel = useState<MethodChannel?>(null);
    final brightness = context.theme.brightness.name;

    useEffect(() {
      final channel = nativeChannel.value;
      if (channel != null) {
        unawaited(channel.invokeMethod<void>('setLabel', label));
        unawaited(channel.invokeMethod<void>('setBrightness', brightness));
      }
      return null;
    }, [nativeChannel.value, label, brightness]);

    return Semantics(
      header: true,
      label: label,
      child: ExcludeSemantics(
        child: SizedBox(
          key: const ValueKey('channel-sticky-date-header-surface'),
          width: width,
          height: height,
          child: UiKitView(
            key: const ValueKey('channel-sticky-date-header-ios-glass'),
            viewType: StickyDateHeader._iosViewType,
            hitTestBehavior: PlatformViewHitTestBehavior.transparent,
            creationParams: <String, Object>{
              'label': label,
              'brightness': brightness,
            },
            creationParamsCodec: const StandardMessageCodec(),
            onPlatformViewCreated: (viewId) {
              nativeChannel.value = MethodChannel(
                '${StickyDateHeader._iosViewType}/$viewId',
              );
            },
          ),
        ),
      ),
    );
  }
}
