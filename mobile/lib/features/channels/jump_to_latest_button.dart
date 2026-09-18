import 'dart:async';
import 'dart:ui';

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter/services.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';

import '../../shared/theme/theme.dart';
import 'message_action_backdrop_state.dart';

/// Shared channel/thread motion for an explicit return-to-latest action.
const jumpToLatestScrollDuration = Duration(milliseconds: 220);
const jumpToLatestScrollCurve = Curves.easeOutCubic;

/// Compact conversation control that returns a detached timeline to its tail.
class JumpToLatestButton extends HookWidget {
  final String id;
  final VoidCallback onPressed;

  const JumpToLatestButton({
    required this.onPressed,
    this.id = 'channel',
    super.key,
  });

  static const _iosViewType = 'buzz/jump_to_latest_glass';

  @override
  Widget build(BuildContext context) {
    final nativeChannel = useState<MethodChannel?>(null);
    final onPressedRef = useRef(onPressed)..value = onPressed;
    final brightness = context.theme.brightness.name;
    final routeIsCurrent = ModalRoute.isCurrentOf(context) ?? true;

    useEffect(() {
      final channel = nativeChannel.value;
      if (channel == null) return null;
      channel.setMethodCallHandler((call) async {
        if (call.method == 'pressed') onPressedRef.value();
      });
      return () => channel.setMethodCallHandler(null);
    }, [nativeChannel.value]);

    useEffect(() {
      final channel = nativeChannel.value;
      if (channel != null) {
        unawaited(channel.invokeMethod<void>('setBrightness', brightness));
      }
      return null;
    }, [nativeChannel.value, brightness]);

    final borderColor = context.colors.onSurface.withValues(alpha: 0.08);
    final usesNativeIosGlass = defaultTargetPlatform == TargetPlatform.iOS;

    Widget buildFlutterSurface() {
      return Material(
        color: Colors.transparent,
        child: InkResponse(
          containedInkWell: true,
          customBorder: const CircleBorder(),
          onTap: onPressed,
          radius: Grid.sm,
          child: Align(
            key: ValueKey('$id-jump-to-latest-visual-anchor'),
            alignment: Alignment.bottomCenter,
            child: ClipOval(
              child: BackdropFilter(
                filter: ImageFilter.blur(sigmaX: 20, sigmaY: 20),
                child: Container(
                  key: ValueKey('$id-jump-to-latest-surface'),
                  width: Grid.lg,
                  height: Grid.lg,
                  decoration: BoxDecoration(
                    color: context.colors.surface.withValues(alpha: 0.72),
                    shape: BoxShape.circle,
                    border: Border.all(color: borderColor),
                  ),
                  child: Icon(
                    LucideIcons.arrowDown,
                    size: Grid.gutter,
                    color: context.colors.onSurfaceVariant,
                  ),
                ),
              ),
            ),
          ),
        ),
      );
    }

    return Semantics(
      button: true,
      label: 'Jump to latest message',
      child: Tooltip(
        excludeFromSemantics: true,
        message: 'Jump to latest message',
        child: SizedBox.square(
          dimension: Grid.xl,
          child: ValueListenableBuilder<bool>(
            valueListenable: messageActionBackdropActive,
            builder: (context, backdropActive, _) {
              if (!routeIsCurrent) return const SizedBox.shrink();
              if (!usesNativeIosGlass || backdropActive) {
                return buildFlutterSurface();
              }
              return SizedBox.expand(
                key: ValueKey('$id-jump-to-latest-surface'),
                child: UiKitView(
                  key: ValueKey('$id-jump-to-latest-ios-glass'),
                  viewType: _iosViewType,
                  hitTestBehavior: PlatformViewHitTestBehavior.opaque,
                  creationParams: <String, Object>{'brightness': brightness},
                  creationParamsCodec: const StandardMessageCodec(),
                  onPlatformViewCreated: (viewId) {
                    nativeChannel.value = MethodChannel(
                      '$_iosViewType/$viewId',
                    );
                  },
                ),
              );
            },
          ),
        ),
      ),
    );
  }
}
