import 'package:buzz/features/channels/channel_filter/channel_filter_store.dart';
import 'package:buzz/features/channels/channel_sections/section_reorder_sheet.dart';
import 'package:buzz/features/channels/channel_sort/channel_sort_storage.dart';
import 'package:buzz/features/channels/channels_page/messages_display.dart';
import 'package:buzz/shared/theme/theme_provider.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  test(
    'section defaults survive upgrade and overrides reopen in their own scope',
    () async {
      SharedPreferences.setMockInitialValues({});
      final prefs = await SharedPreferences.getInstance();
      ProviderContainer open(String scope) {
        final container = ProviderContainer(
          overrides: [
            mobileSectionOrderKeyProvider.overrideWithValue(scope),
            savedPrefsProvider.overrideWithValue(prefs),
          ],
        );
        addTearDown(container.dispose);
        return container;
      }

      final first = open('relay-a:alice');
      final store = ChannelFilterStore.empty().edit(
        'channels',
        ChannelFilterMode.unreads,
        previewCount: 15,
      );
      final inherited = first.read(messagesDisplayProvider);
      expect(inherited.modeFor(store, 'channels'), ChannelFilterMode.unreads);
      expect(inherited.limitFor(store, 'channels'), 15);
      expect(inherited.sort, isNull);
      await first
          .read(messagesDisplayProvider.notifier)
          .update(
            const MessagesDisplay(count: 10, sort: ChannelSortMode.recent),
          );
      final restored = open('relay-a:alice').read(messagesDisplayProvider);
      expect(restored.sort, ChannelSortMode.recent);
      expect(restored.limitFor(store, 'channels'), 10);
      expect(restored.modeFor(store, 'channels'), ChannelFilterMode.unreads);
      for (final scope in ['relay-b:alice', 'relay-a:bob']) {
        expect(open(scope).read(messagesDisplayProvider).count, isNull);
      }
    },
  );
}
