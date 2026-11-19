import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_hooks/flutter_hooks.dart';

/// UIKit's system switch on iOS, with the Material switch elsewhere.
class BuzzSwitch extends HookWidget {
  const BuzzSwitch({
    super.key,
    required this.value,
    required this.label,
    required this.onChanged,
  });

  final bool value;
  final String label;
  final ValueChanged<bool>? onChanged;

  @override
  Widget build(BuildContext context) {
    final channel = useState<MethodChannel?>(null);
    final latest = useRef(onChanged)..value = onChanged;
    final brightness = Theme.of(context).brightness.name;
    final arguments = <String, Object>{
      'value': value,
      'enabled': onChanged != null,
      'label': label,
      'brightness': brightness,
    };
    useEffect(() {
      final current = channel.value;
      if (current == null) return null;
      current.setMethodCallHandler((call) async {
        if (call.method == 'changed' && call.arguments is bool) {
          latest.value?.call(call.arguments as bool);
        }
      });
      return () => current.setMethodCallHandler(null);
    }, [channel.value]);
    useEffect(() {
      final current = channel.value;
      if (current != null) {
        unawaited(current.invokeMethod<void>('update', arguments));
      }
      return null;
    }, [channel.value, value, onChanged != null, label, brightness]);
    if (defaultTargetPlatform != TargetPlatform.iOS) {
      return Switch(value: value, onChanged: onChanged);
    }
    return SizedBox(
      width: 60,
      height: 48,
      child: UiKitView(
        viewType: 'buzz/switch',
        creationParams: arguments,
        creationParamsCodec: const StandardMessageCodec(),
        onPlatformViewCreated: (id) =>
            channel.value = MethodChannel('buzz/switch/$id'),
      ),
    );
  }
}
