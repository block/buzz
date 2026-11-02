import 'package:buzz/features/profile/profile_avatar_sheet_app.dart';
import 'package:buzz/shared/emoji/emoji_avatar.dart';
import 'package:buzz/shared/emoji/emoji_data_provider.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shared_preferences/shared_preferences.dart';

void main() {
  testWidgets('open avatar editor changes theme without losing its draft', (
    tester,
  ) async {
    SharedPreferences.setMockInitialValues({});
    const sheet = MethodChannel('buzz/profile_avatar_sheet');
    final messenger = tester.binding.defaultBinaryMessenger;
    final url = emojiAvatarDataUrl('🐝', 0xFFFFE75C);
    Map? finished;
    final controls = <Map>[];
    messenger.setMockMethodCallHandler(sheet, (call) async {
      if (call.method == 'initial') {
        return {
          'dark': false,
          'draft': {'type': 'url', 'url': url},
          'previewRect': {
            'x': 90.0,
            'y': 200.0,
            'width': 220.0,
            'height': 220.0,
          },
        };
      }
      if (call.method == 'controls') controls.add(call.arguments as Map);
      if (call.method == 'finish') finished = call.arguments as Map?;
      return null;
    });
    addTearDown(() => messenger.setMockMethodCallHandler(sheet, null));
    await tester.runAsync(profileAvatarEditorMain);
    await tester.pump();
    final container = ProviderScope.containerOf(
      tester.element(find.byType(MaterialApp)),
    );
    await tester.runAsync(() => container.read(emojiDatasetProvider.future));
    await tester.pumpAndSettle();
    for (final dark in [true, false]) {
      await messenger.handlePlatformMessage(
        'buzz/profile_avatar_theme',
        const StandardMethodCodec().encodeMethodCall(
          MethodCall('appearance', {'dark': dark}),
        ),
        (_) {},
      );
      await tester.pumpAndSettle();
      expect(
        tester.widget<MaterialApp>(find.byType(MaterialApp)).themeMode,
        dark ? ThemeMode.dark : ThemeMode.light,
      );
      expect(find.text('🐝'), findsWidgets);
    }
    final navigator = tester.state<NavigatorState>(
      find.byType(Navigator).first,
    );
    navigator.push<void>(
      MaterialPageRoute(
        fullscreenDialog: true,
        builder: (_) => const Scaffold(body: Text('Position Photo')),
      ),
    );
    await tester.pumpAndSettle();
    expect(controls.last['fullScreen'], isTrue);
    expect(controls.last['canDone'], isFalse);
    navigator.pop();
    await tester.pumpAndSettle();
    expect(controls.last['fullScreen'], isFalse);
    expect(controls.last['canDone'], isTrue);
    await messenger.handlePlatformMessage(
      sheet.name,
      const StandardMethodCodec().encodeMethodCall(const MethodCall('done')),
      (_) {},
    );
    await tester.pumpAndSettle();
    expect(finished, {'type': 'url', 'url': url});
    await tester.pumpWidget(const SizedBox.shrink());
  });
}
