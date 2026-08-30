import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart' show PlatformViewHitTestBehavior;
import 'package:flutter/services.dart';
import 'package:flutter_hooks/flutter_hooks.dart';

/// A platform-native iOS segmented control embedded in Flutter.
class IosNativeSegmentedControl extends HookWidget {
  /// Creates a native segmented control for the supplied labels.
  const IosNativeSegmentedControl({
    super.key,
    required this.items,
    required this.selectedIndex,
    required this.onChanged,
    this.height = 40,
  });

  /// The registered iOS platform-view identifier.
  static const viewType = 'buzz/native_segmented_control';

  /// Labels displayed by the native segments.
  final List<String> items;

  /// The selected segment index.
  final int selectedIndex;

  /// Called with a new index, or null when the control is disabled.
  final ValueChanged<int>? onChanged;

  /// The height of the embedded native control.
  final double height;

  @override
  Widget build(BuildContext context) {
    assert(defaultTargetPlatform == TargetPlatform.iOS);
    final nativeChannel = useState<MethodChannel?>(null);
    final onChangedRef = useRef(onChanged)..value = onChanged;
    final brightness = Theme.of(context).brightness.name;
    final enabled = onChanged != null;

    useEffect(() {
      final channel = nativeChannel.value;
      if (channel == null) return null;
      channel.setMethodCallHandler((call) async {
        if (call.method == 'changed' && call.arguments is int) {
          onChangedRef.value?.call(call.arguments as int);
        }
      });
      return () => channel.setMethodCallHandler(null);
    }, [nativeChannel.value]);

    useEffect(() {
      final channel = nativeChannel.value;
      if (channel != null) {
        unawaited(
          channel.invokeMethod<void>('setState', <String, Object>{
            'selectedIndex': selectedIndex,
            'brightness': brightness,
            'enabled': enabled,
          }),
        );
      }
      return null;
    }, [nativeChannel.value, selectedIndex, brightness, enabled]);

    return SizedBox(
      height: height,
      child: UiKitView(
        viewType: viewType,
        hitTestBehavior: PlatformViewHitTestBehavior.opaque,
        creationParams: <String, Object>{
          'items': items,
          'selectedIndex': selectedIndex,
          'brightness': brightness,
          'enabled': enabled,
        },
        creationParamsCodec: const StandardMessageCodec(),
        onPlatformViewCreated: (viewId) {
          nativeChannel.value = MethodChannel('$viewType/$viewId');
        },
      ),
    );
  }
}
