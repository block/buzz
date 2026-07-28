import 'package:buzz/features/profile/set_status_sheet.dart';
import 'package:buzz/features/profile/user_status.dart';
import 'package:buzz/features/profile/user_status_provider.dart';
import 'package:buzz/shared/custom_emoji/custom_emoji_provider.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';

import '../../helpers/widget_helpers.dart';

void main() {
  testWidgets('removes only the emoji from an existing status', (tester) async {
    final statusNotifier = _RecordingUserStatusNotifier();
    await tester.pumpWidget(
      WidgetHelpers.testable(
        overrides: [
          customEmojiListProvider.overrideWithValue(const []),
          userStatusProvider.overrideWith(() => statusNotifier),
        ],
        child: Builder(
          builder: (context) => FilledButton(
            onPressed: () => showSetStatusSheet(
              context,
              currentStatus: const UserStatus(
                text: 'Focusing',
                emoji: '\u{1F3AF}',
                updatedAt: 1,
              ),
            ),
            child: const Text('Open status editor'),
          ),
        ),
      ),
    );

    await tester.tap(find.text('Open status editor'));
    await tester.pumpAndSettle();

    expect(find.text('\u{1F3AF}'), findsOneWidget);
    expect(find.byTooltip('Remove status emoji'), findsOneWidget);

    await tester.tap(find.byTooltip('Remove status emoji'));
    await tester.pump();

    expect(find.text('\u{1F3AF}'), findsNothing);
    expect(find.byIcon(LucideIcons.smilePlus), findsOneWidget);
    expect(
      tester.widget<TextField>(find.byType(TextField)).controller?.text,
      'Focusing',
    );

    await tester.tap(find.text('Save'));
    await tester.pumpAndSettle();

    expect(statusNotifier.savedText, 'Focusing');
    expect(statusNotifier.savedEmoji, isEmpty);
  });
}

class _RecordingUserStatusNotifier extends UserStatusNotifier {
  String? savedText;
  String? savedEmoji;

  @override
  Future<UserStatus?> build() async => null;

  @override
  Future<void> setStatus(String text, String emoji) async {
    savedText = text;
    savedEmoji = emoji;
  }
}
