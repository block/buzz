import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';

Future<ProviderContainer> _container(Map<String, Object> initial) async {
  SharedPreferences.setMockInitialValues(initial);
  final prefs = await SharedPreferences.getInstance();
  final container = ProviderContainer(
    overrides: [savedPrefsProvider.overrideWithValue(prefs)],
  );
  addTearDown(container.dispose);
  return container;
}

void main() {
  test('defaults to System, even when the synced theme is pinned', () async {
    final container = await _container({
      'buzz_theme_mode': ThemeMode.light.name,
      'buzz-local-theme.v1':
          '{"version":1,"theme":"buzz","accent":"#3b82f6","followSystem":false}',
    });

    expect(container.read(appearanceModeProvider), ThemeMode.system);
  });

  test('reads and saves the device mode', () async {
    final container = await _container({
      appearanceModeKey: ThemeMode.dark.name,
    });
    expect(container.read(appearanceModeProvider), ThemeMode.dark);

    container.read(appearanceModeProvider.notifier).setMode(ThemeMode.light);

    expect(container.read(appearanceModeProvider), ThemeMode.light);
    final prefs = container.read(savedPrefsProvider);
    expect(prefs.getString(appearanceModeKey), ThemeMode.light.name);
  });

  test('an unknown saved value falls back to System', () async {
    final container = await _container({appearanceModeKey: 'sepia'});

    expect(container.read(appearanceModeProvider), ThemeMode.system);
  });
}
