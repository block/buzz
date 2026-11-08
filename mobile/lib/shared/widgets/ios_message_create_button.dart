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
    this.sectionTitles = const {},
    this.icon = 'plus',
    this.accessibilityLabel = 'Create',
    this.width = 64,
    this.height = 48,
    this.plain = true,
    this.fillWidth = false,
    this.enabled = true,
    this.selectedIndex,
    this.eagerGestures = true,
    this.deferMenuActions = true,
    this.showIconWithLabel = false,
    this.onMenuVisibilityChanged,
    this.mergingNavigation = false,
  });

  final Set<int> dividerBefore;
  final Map<int, String> sectionTitles;
  final String? label;
  final double? labelFontSize;
  final String? avatarUrl, avatarInitial;
  final String icon;
  final String accessibilityLabel;
  final double width, height;
  final bool plain;
  final bool fillWidth;

  /// Whether the native trigger and its actions are interactive.
  final bool enabled;

  /// Option marked with the native menu selection checkmark.
  final int? selectedIndex;

  /// Capture gestures immediately; disable for rows inside a scroll view.
  final bool eagerGestures;

  /// Wait for menu dismissal before actions that navigate to another route.
  final bool deferMenuActions;
  final bool showIconWithLabel;
  final ValueChanged<bool>? onMenuVisibilityChanged;

  /// Hosts the Search/New/Bestie glass surfaces in one native container.
  final bool mergingNavigation;

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
    final latestVisibility = useRef(onMenuVisibilityChanged)
      ..value = onMenuVisibilityChanged;
    final latestEnabled = useRef(enabled)..value = enabled;
    final menu = [
      for (var i = 0; i < actions.length; i++)
        {
          'id': i,
          'dividerBefore': dividerBefore.contains(i),
          'sectionTitle': ?sectionTitles[i],
          'label': actions[i].label,
          'symbol': actions[i].symbol,
          'enabled': enabled && actions[i].onPressed != null,
          'checked': i == selectedIndex,
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
      'deferMenuActions': deferMenuActions,
    };
    // Keep one handler per channel. Replacing this effect on enabled changes
    // would let the old hook cleanup clear the newly installed handler.
    useEffect(() {
      final current = channel.value;
      if (current == null) return null;
      current.setMethodCallHandler((call) async {
        if (call.method == 'menuVisibilityChanged') {
          latestVisibility.value?.call(call.arguments as bool);
          return;
        }
        if (call.method != 'menuAction') return;
        final index = call.arguments as int;
        if (index >= 0 && index < latest.value.length) {
          if (latestEnabled.value) latest.value[index].onPressed?.call();
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
              'enabled': enabled,
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
        enabled,
        selectedIndex,
        deferMenuActions,
        showIconWithLabel,
        ...dividerBefore,
        ...sectionTitles.entries.expand((e) => [e.key, e.value]),
        ...actions.expand((a) => [a.label, a.symbol, a.onPressed != null]),
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
          if (eagerGestures)
            Factory<EagerGestureRecognizer>(EagerGestureRecognizer.new),
        },
        creationParams: {
          ...content,
          'plain': plain,
          'fillWidth': fillWidth,
          'buttonCenterX': width / 2,
          'controlWidth': mergingNavigation ? 112.0 : width,
          'mergingNavigation': mergingNavigation,
          'controlSize': height,
          'hitTargetWidth': width,
          'hitTargetHeight': height,
          'brightness': brightness.name,
          'foregroundColor': foreground,
          'enabled': enabled,
        },
        creationParamsCodec: const StandardMessageCodec(),
        onPlatformViewCreated: (id) =>
            channel.value = MethodChannel('buzz/navigation_glass/$id'),
      ),
    );
  }
}
