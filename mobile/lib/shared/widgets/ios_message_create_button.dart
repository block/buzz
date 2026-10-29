import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/gestures.dart';
import 'package:flutter/services.dart';
import 'package:flutter_hooks/flutter_hooks.dart';

import '../theme/theme.dart';
import 'avatar_image.dart';

/// The four desktop creation destinations, presented by UIKit on iOS.
class IosMessageCreateButton extends HookWidget {
  const IosMessageCreateButton({
    super.key,
    required this.actions,
    this.label,
    this.labelFontSize,
    this.avatarUrl,
    this.avatarInitial,
    this.dividerBefore = const {},
    this.icon = 'plus',
    this.accessibilityLabel = 'Create',
    this.width = 64,
    this.height = 48,
    this.plain = true,
    this.showIconWithLabel = false,
  });

  final Set<int> dividerBefore;
  final String? label;
  final double? labelFontSize;
  final String? avatarUrl, avatarInitial;
  final String icon;
  final String accessibilityLabel;
  final double width, height;
  final bool plain;
  final bool showIconWithLabel;

  final List<({String label, String symbol, VoidCallback? onPressed})> actions;

  @override
  Widget build(BuildContext context) {
    final route = ModalRoute.of(context);
    final coveringProgress = useAnimation(
      route?.secondaryAnimation ?? kAlwaysDismissedAnimation,
    );
    final nativeControlsVisible =
        (route?.isCurrent ?? true) && coveringProgress == 0;

    final brightness = context.theme.brightness;
    final foreground = context.colors.onSurface.toARGB32();
    final avatarFuture = useMemoized(
      () => avatarInitial == null
          ? null
          : nativeAvatarImage(
              url: avatarUrl,
              initial: avatarInitial!,
              circular: true,
              background: context.colors.primaryContainer,
              foreground: context.colors.onPrimaryContainer,
              networkImage: NetworkImage.new,
            ),
      [
        avatarUrl,
        avatarInitial,
        context.colors.primaryContainer,
        context.colors.onPrimaryContainer,
      ],
    );
    final avatar = useFuture(avatarFuture, preserveState: false).data;
    final channel = useState<MethodChannel?>(null);
    final latest = useRef(actions)..value = actions;
    final menu = [
      for (var i = 0; i < actions.length; i++)
        {
          'id': i,
          'dividerBefore': dividerBefore.contains(i),
          'label': actions[i].label,
          'symbol': actions[i].symbol,
          'enabled': actions[i].onPressed != null,
        },
    ];
    final content = <String, Object>{
      'icon': icon,
      'imageBytes': ?avatar,
      'label': ?label,
      'labelFontSize': ?labelFontSize,
      'showIconWithLabel': showIconWithLabel,
      'accessibilityLabel': accessibilityLabel,
      'menu': menu,
    };
    useEffect(() {
      final current = channel.value;
      if (current == null) return null;
      current.setMethodCallHandler((call) async {
        if (call.method != 'menuAction') return;
        final index = call.arguments as int;
        if (index >= 0 && index < latest.value.length) {
          latest.value[index].onPressed?.call();
        }
      });
      return () => current.setMethodCallHandler(null);
    }, [channel.value]);
    useEffect(
      () {
        final current = channel.value;
        if (current != null) {
          unawaited(current.invokeMethod<void>('setContent', content));
          unawaited(
            current.invokeMethod<void>('setAppearance', {
              'brightness': brightness.name,
              'foregroundColor': foreground,
              'enabled': true,
            }),
          );
        }
        return null;
      },
      [
        channel.value,
        avatar,
        icon,
        label,
        labelFontSize,
        accessibilityLabel,
        ...actions.map((a) => a.onPressed != null),
        brightness,
        foreground,
      ],
    );
    // UIKit glass layers can float above the incoming Flutter route during
    // its transition. Remove the covered route's platform view immediately.
    if (!nativeControlsVisible) {
      return SizedBox(width: width, height: height);
    }
    return SizedBox(
      width: width,
      height: height,
      child: UiKitView(
        viewType: 'buzz/navigation_glass',
        gestureRecognizers: {
          Factory<EagerGestureRecognizer>(EagerGestureRecognizer.new),
        },
        creationParams: {
          ...content,
          'plain': plain,
          'buttonCenterX': width / 2,
          'controlWidth': width,
          'controlSize': height,
          'hitTargetWidth': width,
          'hitTargetHeight': height,
          'brightness': brightness.name,
          'foregroundColor': foreground,
          'enabled': true,
        },
        creationParamsCodec: const StandardMessageCodec(),
        onPlatformViewCreated: (id) =>
            channel.value = MethodChannel('buzz/navigation_glass/$id'),
      ),
    );
  }
}
