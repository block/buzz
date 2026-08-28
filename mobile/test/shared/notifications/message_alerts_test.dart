import 'package:buzz/shared/notifications/message_alerts.dart';
import 'package:buzz/shared/theme/theme_provider.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';

void main() {
  test('all-messages alerts default to on', () async {
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();
    final container = ProviderContainer(
      overrides: [savedPrefsProvider.overrideWithValue(prefs)],
    );
    addTearDown(container.dispose);

    expect(container.read(notificationPreferencesProvider).allMessages, isTrue);
  });

  test('all-messages alerts persist the off state', () async {
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();
    final container = ProviderContainer(
      overrides: [savedPrefsProvider.overrideWithValue(prefs)],
    );
    addTearDown(container.dispose);

    await container
        .read(notificationPreferencesProvider.notifier)
        .setAllMessages(false);
    expect(
      container.read(notificationPreferencesProvider).allMessages,
      isFalse,
    );
    expect(prefs.getBool('buzz-notification-all-messages'), isFalse);
  });
}
