import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart' show PlatformViewHitTestBehavior;
import 'package:flutter/services.dart';
import 'package:flutter_hooks/flutter_hooks.dart';

/// Native iOS liquid-glass pagination that supports tapping and scrubbing.
class IosGlassThemePagination extends HookWidget {
  /// Creates a native theme-pagination control.
  const IosGlassThemePagination({
    super.key,
    required this.count,
    required this.selected,
    required this.animateChanges,
    required this.onSelected,
    required this.activeColor,
    required this.inactiveColor,
  });

  static const viewType = 'buzz/theme_pagination_glass';

  final int count;
  final int selected;
  final bool animateChanges;
  final ValueChanged<int> onSelected;
  final Color activeColor;
  final Color inactiveColor;

  @override
  Widget build(BuildContext context) {
    assert(defaultTargetPlatform == TargetPlatform.iOS);
    final nativeChannel = useState<MethodChannel?>(null);
    final onSelectedRef = useRef(onSelected)..value = onSelected;
    final brightness = Theme.of(context).brightness.name;
    final activeColorValue = activeColor.toARGB32();
    final inactiveColorValue = inactiveColor.toARGB32();

    useEffect(() {
      final channel = nativeChannel.value;
      if (channel == null) return null;
      channel.setMethodCallHandler((call) async {
        if (call.method == 'selected' && call.arguments is int) {
          onSelectedRef.value(call.arguments as int);
        }
      });
      return () => channel.setMethodCallHandler(null);
    }, [nativeChannel.value]);

    useEffect(
      () {
        final channel = nativeChannel.value;
        if (channel != null) {
          unawaited(
            channel.invokeMethod<void>('setState', <String, Object>{
              'count': count,
              'selected': selected,
              'animateChanges': animateChanges,
              'brightness': brightness,
              'activeColor': activeColorValue,
              'inactiveColor': inactiveColorValue,
            }),
          );
        }
        return null;
      },
      [
        nativeChannel.value,
        count,
        selected,
        animateChanges,
        brightness,
        activeColorValue,
        inactiveColorValue,
      ],
    );

    return UiKitView(
      viewType: viewType,
      hitTestBehavior: PlatformViewHitTestBehavior.opaque,
      creationParams: <String, Object>{
        'count': count,
        'selected': selected,
        'animateChanges': animateChanges,
        'brightness': brightness,
        'activeColor': activeColorValue,
        'inactiveColor': inactiveColorValue,
      },
      creationParamsCodec: const StandardMessageCodec(),
      onPlatformViewCreated: (viewId) {
        nativeChannel.value = MethodChannel('$viewType/$viewId');
      },
    );
  }
}
