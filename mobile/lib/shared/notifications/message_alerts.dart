import 'package:flutter/services.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../theme/theme_provider.dart';

const _allMessagesKey = 'buzz-notification-all-messages';

/// Preference for sounding and posting a local notification on every live
/// message. Defaults to on so a fresh install matches desktop All messages.
class NotificationPreferences {
  const NotificationPreferences({this.allMessages = true});

  final bool allMessages;
}

class NotificationPreferencesNotifier extends Notifier<NotificationPreferences> {
  @override
  NotificationPreferences build() {
    final prefs = ref.watch(savedPrefsProvider);
    return NotificationPreferences(
      allMessages: prefs.getBool(_allMessagesKey) ?? true,
    );
  }

  Future<void> setAllMessages(bool enabled) async {
    final prefs = ref.read(savedPrefsProvider);
    await prefs.setBool(_allMessagesKey, enabled);
    state = NotificationPreferences(allMessages: enabled);
    if (enabled) {
      await MessageAlerts.requestPermission();
    }
  }
}

final notificationPreferencesProvider =
    NotifierProvider<NotificationPreferencesNotifier, NotificationPreferences>(
      NotificationPreferencesNotifier.new,
    );

/// Native seam for Android local notifications with sound.
class MessageAlerts {
  MessageAlerts._();

  static const _channel = MethodChannel('buzz/message_alerts');

  static Future<bool> requestPermission() async {
    try {
      return await _channel.invokeMethod<bool>('requestPermission') ?? false;
    } on MissingPluginException {
      return false;
    } on PlatformException {
      return false;
    }
  }

  static Future<void> show({
    required String title,
    required String body,
  }) async {
    final trimmed = body.trim();
    try {
      await _channel.invokeMethod<void>('show', {
        'title': title,
        'body': trimmed.isEmpty ? 'New message' : trimmed,
      });
    } on MissingPluginException {
      return;
    } on PlatformException {
      return;
    }
  }
}
