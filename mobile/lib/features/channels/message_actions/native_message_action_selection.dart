import 'package:flutter/services.dart';
import 'package:flutter_hooks/flutter_hooks.dart';

/// Receives selections from the native message menu while [viewId] is mounted.
void useNativeMessageActionSelection(
  int? viewId,
  ValueChanged<String> onSelected,
) {
  final latestOnSelected = useRef(onSelected);
  latestOnSelected.value = onSelected;

  // Callback changes must not replace this effect: the previous hook's cleanup
  // can clear the new handler on the same channel after a keyboard-driven rebuild.
  useEffect(() {
    if (viewId == null) return null;
    final channel = MethodChannel('buzz/native_message_action_surface/$viewId');
    channel.setMethodCallHandler((call) async {
      if (call.method != 'selected' || call.arguments is! Map) return;
      final actionId = (call.arguments as Map)['id'];
      if (actionId is String) latestOnSelected.value(actionId);
    });
    return () => channel.setMethodCallHandler(null);
  }, [viewId]);
}
