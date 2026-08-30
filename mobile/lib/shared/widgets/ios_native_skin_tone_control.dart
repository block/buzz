import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart' show PlatformViewHitTestBehavior;
import 'package:flutter/services.dart';
import 'package:flutter_hooks/flutter_hooks.dart';

/// A native iOS glass menu for selecting an emoji skin tone.
class IosNativeSkinToneControl extends HookWidget {
  /// Creates a native skin-tone selection menu.
  const IosNativeSkinToneControl({
    super.key,
    required this.value,
    required this.colors,
    required this.labels,
    required this.onChanged,
  });

  /// The registered iOS platform-view identifier.
  static const viewType = 'buzz/native_skin_tone_control';

  /// The selected skin-tone index.
  final int value;

  /// Colors displayed for the available tone options.
  final List<Color> colors;

  /// Accessible labels corresponding to [colors].
  final List<String> labels;

  /// Called with the newly selected tone index.
  final ValueChanged<int> onChanged;

  @override
  Widget build(BuildContext context) {
    assert(defaultTargetPlatform == TargetPlatform.iOS);
    final nativeChannel = useState<MethodChannel?>(null);
    final onChangedRef = useRef(onChanged)..value = onChanged;

    useEffect(() {
      final channel = nativeChannel.value;
      if (channel == null) return null;
      channel.setMethodCallHandler((call) async {
        if (call.method == 'changed' && call.arguments is int) {
          onChangedRef.value(call.arguments as int);
        }
      });
      return () => channel.setMethodCallHandler(null);
    }, [nativeChannel.value]);

    useEffect(() {
      final channel = nativeChannel.value;
      if (channel != null) {
        unawaited(channel.invokeMethod<void>('setValue', value));
      }
      return null;
    }, [nativeChannel.value, value]);

    return SizedBox.square(
      dimension: 48,
      child: UiKitView(
        viewType: viewType,
        hitTestBehavior: PlatformViewHitTestBehavior.opaque,
        creationParams: <String, Object>{
          'value': value,
          'colors': [for (final color in colors) color.toARGB32()],
          'labels': labels,
          'brightness': Theme.of(context).brightness.name,
        },
        creationParamsCodec: const StandardMessageCodec(),
        onPlatformViewCreated: (viewId) {
          nativeChannel.value = MethodChannel('$viewType/$viewId');
        },
      ),
    );
  }
}
