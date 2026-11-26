import 'package:buzz/features/channels/channel_actions_sheet.dart';
import 'package:buzz/features/channels/channel_sections/channel_sections_storage.dart';
import 'package:buzz/features/channels/channels_page.dart';
import 'package:buzz/shared/custom_emoji/custom_emoji_provider.dart';
import 'package:buzz/shared/emoji/emoji_data.dart';
import 'package:buzz/shared/emoji/emoji_data_provider.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/theme_provider.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';

void main() {
  testWidgets('create and clear a section emoji without renaming it', (
    tester,
  ) async {
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();
    const fire = EmojiEntry(
      id: 'fire',
      name: 'Fire',
      keywords: [],
      native: '🔥',
      categoryId: 'nature',
    );
    ChannelSection? section;
    await tester.pumpWidget(
      ProviderScope(
        overrides: [
          savedPrefsProvider.overrideWithValue(prefs),
          myPubkeyProvider.overrideWithValue('self'),
          customEmojiListProvider.overrideWithValue(const []),
          emojiDatasetProvider.overrideWith(
            (ref) async => const EmojiDataset(
              categories: [
                EmojiCategory(id: 'nature', emoji: [fire]),
              ],
              all: [fire],
              nativeToShortcode: {'🔥': ':fire:'},
            ),
          ),
        ],
        child: MaterialApp(
          home: Scaffold(
            body: Builder(
              builder: (context) => TextButton(
                onPressed: () async {
                  final existing = section;
                  final edit = existing == null
                      ? await showChannelSectionDialog(
                          context,
                          inScope: () => true,
                        )
                      : await showSectionRenameEditor(context, existing);
                  if (edit != null) {
                    section = ChannelSection(
                      id: 'design',
                      name: edit.name,
                      icon: edit.icon,
                      order: 0,
                    );
                  }
                },
                child: const Text('Open section'),
              ),
            ),
          ),
        ),
      ),
    );

    await tester.tap(find.text('Open section'));
    await tester.pumpAndSettle();
    await tester.enterText(find.byType(TextField), 'Design');
    await tester.tap(find.byTooltip('Choose section emoji'));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const ValueKey('emoji-tile-fire')));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Create'));
    await tester.pumpAndSettle();
    expect(section?.icon, '🔥');

    await tester.tap(find.text('Open section'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Remove emoji'));
    await tester.tap(find.text('Cancel'));
    await tester.pumpAndSettle();
    expect(section?.icon, '🔥');

    await tester.tap(find.text('Open section'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Remove emoji'));
    await tester.tap(find.text('Rename'));
    await tester.pumpAndSettle();
    expect(section?.name, 'Design');
    expect(section?.icon, isNull);
  });
}
