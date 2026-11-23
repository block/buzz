import 'package:buzz/features/channels/channel_sections/collapsed_section_provider.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';

void main() {
  test(
    'restores toggles across launches and keeps sidebar scopes separate',
    () async {
      SharedPreferences.setMockInitialValues({});
      final sidebar = collapsedSectionsProvider('relay:account');
      final first = ProviderContainer();
      addTearDown(first.dispose);
      final sections = first.read(sidebar.notifier);
      await Future.wait([
        sections.toggle('channels'),
        sections.toggle('custom:design'),
      ]);
      await sections.toggle('channels');

      final relaunched = ProviderContainer();
      addTearDown(relaunched.dispose);
      expect(await relaunched.read(sidebar.future), {'custom:design'});
      expect(
        await relaunched.read(collapsedSectionsProvider('other').future),
        isEmpty,
      );
    },
  );
}
