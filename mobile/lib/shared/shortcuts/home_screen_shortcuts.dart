import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';

/// Home Screen quick actions offered by the iOS app icon.
enum HomeScreenShortcut {
  newChat('new-chat'),
  newDm('new-dm'),
  messageBestie('message-bestie');

  const HomeScreenShortcut(this.type);

  /// The `UIApplicationShortcutItem.type` shared with the native bridge.
  final String type;
}

const _channel = MethodChannel('buzz/home_screen_shortcuts');

/// The latest tapped shortcut that Home has not run yet.
final pendingHomeScreenShortcut = ValueNotifier<HomeScreenShortcut?>(null);

HomeScreenShortcut? _shortcut(Object? type) =>
    HomeScreenShortcut.values.where((s) => s.type == type).firstOrNull;

/// Starts receiving taps and collects one that launched or woke the app
/// before Dart was listening.
Future<void> listenForHomeScreenShortcuts() async {
  if (defaultTargetPlatform != TargetPlatform.iOS) return;
  _channel.setMethodCallHandler((call) async {
    if (call.method != 'open') throw MissingPluginException(call.method);
    final shortcut = _shortcut(call.arguments);
    if (shortcut == null) return null;
    pendingHomeScreenShortcut.value = shortcut;
    return 'handled';
  });
  try {
    final shortcut = _shortcut(
      await _channel.invokeMethod<String>('takePending'),
    );
    if (shortcut != null) pendingHomeScreenShortcut.value = shortcut;
  } on MissingPluginException {
    // Flutter tests and non-Runner embeddings do not install the native bridge.
  }
}

/// Replaces the app icon's quick actions, in menu order.
Future<void> publishHomeScreenShortcuts(
  List<({HomeScreenShortcut shortcut, String title})> items,
) async {
  if (defaultTargetPlatform != TargetPlatform.iOS) return;
  try {
    await _channel.invokeMethod<void>('setItems', [
      for (final item in items)
        {'type': item.shortcut.type, 'title': item.title},
    ]);
  } on MissingPluginException {
    // Flutter tests and non-Runner embeddings do not install the native bridge.
  }
}
