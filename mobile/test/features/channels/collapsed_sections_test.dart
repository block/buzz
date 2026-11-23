import 'package:buzz/features/channels/channel_sections/collapsed_section_provider.dart';
import 'package:buzz/shared/theme/theme_provider.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';

void main() {
  test(
    'restores toggles across launches and keeps sidebar scopes separate',
    () async {
      SharedPreferences.setMockInitialValues({});
      final prefs = await SharedPreferences.getInstance();
      ProviderContainer launch() {
        final container = ProviderContainer(
          overrides: [savedPrefsProvider.overrideWithValue(prefs)],
        );
        addTearDown(container.dispose);
        return container;
      }

      final sidebar = collapsedSectionsProvider('relay:account');
      final sections = launch().read(sidebar.notifier);
      await Future.wait([
        sections.toggle('channels'),
        sections.toggle('custom:design'),
      ]);
      await sections.toggle('channels');

      final relaunched = launch();
      expect(relaunched.read(sidebar), {'custom:design'});
      expect(relaunched.read(collapsedSectionsProvider('other')), isEmpty);
    },
  );
}
