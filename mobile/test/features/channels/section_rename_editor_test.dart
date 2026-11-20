import 'dart:async';

import 'package:buzz/features/channels/channel_sections/channel_sections_storage.dart';
import 'package:buzz/features/channels/channels_page.dart';
import 'package:buzz/shared/custom_emoji/custom_emoji.dart';
import 'package:buzz/shared/custom_emoji/custom_emoji_provider.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

class _PendingPalette extends CustomEmojiPaletteNotifier {
  _PendingPalette(this.pending);
  final Future<List<CustomEmoji>> pending;

  @override
  Future<List<CustomEmoji>> build() => pending;
}

const section = ChannelSection(id: 'design', name: 'Design', order: 0);

void main() {
  testWidgets('closing Rename cancels a waiting iOS picker', (tester) async {
    final palette = Completer<List<CustomEmoji>>();
    final editor = Completer<Map<String, dynamic>?>();
    final messenger = tester.binding.defaultBinaryMessenger;
    const editorChannel = MethodChannel('buzz/profile_text_editor');
    const emojiChannel = MethodChannel('buzz/native_emoji_picker');
    messenger.setMockMethodCallHandler(editorChannel, (_) => editor.future);
    var presentations = 0;
    messenger.setMockMethodCallHandler(emojiChannel, (_) async {
      presentations++;
      return null;
    });
    addTearDown(() {
      messenger.setMockMethodCallHandler(editorChannel, null);
      messenger.setMockMethodCallHandler(emojiChannel, null);
    });

    await tester.pumpWidget(
      ProviderScope(
        overrides: [
          customEmojiPaletteProvider.overrideWith(
            () => _PendingPalette(palette.future),
          ),
        ],
        child: const MaterialApp(home: Scaffold()),
      ),
    );
    final edit = showSectionRenameEditor(
      tester.element(find.byType(Scaffold)),
      section,
    );
    final choose = messenger.handlePlatformMessage(
      'buzz/section_emoji',
      const StandardMethodCodec().encodeMethodCall(const MethodCall('choose')),
      (_) {},
    );
    await tester.pump();
    editor.complete(null);
    await tester.pump();
    expect(await edit, isNull);
    palette.complete([]);
    await tester.pumpAndSettle();
    await choose;
    expect(presentations, 0);
  }, variant: TargetPlatformVariant.only(TargetPlatform.iOS));

  testWidgets('the Android name field grows with large text', (tester) async {
    await tester.pumpWidget(
      MaterialApp(
        builder: (context, child) => MediaQuery(
          data: MediaQuery.of(
            context,
          ).copyWith(textScaler: TextScaler.linear(2.5)),
          child: child!,
        ),
        home: const Scaffold(),
      ),
    );
    final edit = showSectionRenameEditor(
      tester.element(find.byType(Scaffold)),
      section,
    );
    await tester.pumpAndSettle();
    expect(tester.getSize(find.byType(TextField)).height, greaterThan(52));
    await tester.enterText(find.byType(TextField), 'New name');
    await tester.tap(find.text('Rename'));
    await tester.pumpAndSettle();
    expect((await edit)?.name, 'New name');
  }, variant: TargetPlatformVariant.only(TargetPlatform.android));
}
