import 'package:buzz/shared/notifications/agent_live_update_preferences.dart';
import 'package:buzz/shared/theme/theme_provider.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';

void main() {
  test('agent Live Updates are enabled by default', () async {
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();
    final container = ProviderContainer(
      overrides: [savedPrefsProvider.overrideWithValue(prefs)],
    );
    addTearDown(container.dispose);

    expect(container.read(agentLiveUpdatesEnabledProvider), isTrue);
  });

  test('disabled preference persists across provider containers', () async {
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();
    final firstContainer = ProviderContainer(
      overrides: [savedPrefsProvider.overrideWithValue(prefs)],
    );

    firstContainer
        .read(agentLiveUpdatesEnabledProvider.notifier)
        .setEnabled(false);
    expect(firstContainer.read(agentLiveUpdatesEnabledProvider), isFalse);
    firstContainer.dispose();

    final secondContainer = ProviderContainer(
      overrides: [savedPrefsProvider.overrideWithValue(prefs)],
    );
    addTearDown(secondContainer.dispose);

    expect(secondContainer.read(agentLiveUpdatesEnabledProvider), isFalse);
  });
}
