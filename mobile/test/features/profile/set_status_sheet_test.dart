import 'package:buzz/features/profile/set_status_sheet.dart';
import 'package:buzz/features/profile/user_status.dart';
import 'package:buzz/features/profile/user_status_provider.dart';
import 'package:buzz/shared/custom_emoji/custom_emoji_provider.dart';
import 'package:flutter/cupertino.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';

import '../../helpers/widget_helpers.dart';

void main() {
  testWidgets('keeps its resting top edge when the keyboard appears', (
    tester,
  ) async {
    addTearDown(tester.view.reset);
    await tester.pumpWidget(
      WidgetHelpers.testable(
        overrides: [
          customEmojiListProvider.overrideWithValue(const []),
          userStatusProvider.overrideWith(() => _RecordingUserStatusNotifier()),
        ],
        child: Builder(
          builder: (context) => FilledButton(
            onPressed: () => showSetStatusSheet(context),
            child: const Text('Open status editor'),
          ),
        ),
      ),
    );

    await tester.tap(find.text('Open status editor'));
    await tester.pumpAndSettle();
    final restingTop = tester.getTopLeft(find.byType(BottomSheet)).dy;

    tester.view.viewInsets = const FakeViewPadding(bottom: 300);
    await tester.pumpAndSettle();

    expect(
      tester.getTopLeft(find.byType(BottomSheet)).dy,
      closeTo(restingTop, 0.01),
    );
  });

  testWidgets('uses Buzz rows, desktop presets, and a real duration', (
    tester,
  ) async {
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
    expect(find.byKey(const ValueKey('status-input-outline')), findsOneWidget);
    expect(find.text('Visible in chats to everyone'), findsNothing);
    expect(find.text('Let others know what you\u2019re up to.'), findsNothing);
    expect(find.text('1 day'), findsOneWidget);
    expect(find.text('In a meeting'), findsOneWidget);
    expect(find.text('Commuting'), findsOneWidget);
    expect(find.text('Out sick'), findsOneWidget);
    expect(find.text('Vacationing'), findsOneWidget);
    expect(find.text('Working remotely'), findsOneWidget);

    await tester.tap(find.text('1 day'));
    await tester.pumpAndSettle();
    expect(find.text('1 hour'), findsOneWidget);
    expect(find.text('8 hours'), findsOneWidget);
    expect(find.text('1 week'), findsOneWidget);
    expect(find.text('Custom'), findsOneWidget);
    await tester.tap(find.text('8 hours'));
    await tester.pumpAndSettle();
    expect(find.text('8 hours'), findsOneWidget);

    await tester.tap(find.byTooltip('Remove status emoji'));
    await tester.pump();

    expect(find.text('\u{1F3AF}'), findsNothing);
    expect(find.byIcon(LucideIcons.smilePlus), findsOneWidget);
    expect(
      tester.widget<TextField>(find.byType(TextField)).controller?.text,
      'Focusing',
    );

    final beforeSave = DateTime.now();
    await tester.tap(find.byTooltip('Save status'));
    await tester.pumpAndSettle();

    expect(statusNotifier.savedText, 'Focusing');
    expect(statusNotifier.savedEmoji, isEmpty);
    expect(statusNotifier.savedExpiresAt, isNotNull);
    expect(
      statusNotifier.savedExpiresAt!.difference(beforeSave).inSeconds,
      inInclusiveRange(
        const Duration(hours: 8).inSeconds - 2,
        const Duration(hours: 8).inSeconds + 2,
      ),
    );
  });

  testWidgets('opens with an out-of-range remote expiration', (tester) async {
    await tester.pumpWidget(
      WidgetHelpers.testable(
        overrides: [
          customEmojiListProvider.overrideWithValue(const []),
          userStatusProvider.overrideWith(() => _RecordingUserStatusNotifier()),
        ],
        child: Builder(
          builder: (context) => FilledButton(
            onPressed: () => showSetStatusSheet(
              context,
              currentStatus: const UserStatus(
                text: 'Focusing',
                emoji: '',
                updatedAt: 1,
                expiresAt: 9_000_000_000_000,
              ),
            ),
            child: const Text('Open status editor'),
          ),
        ),
      ),
    );

    await tester.tap(find.text('Open status editor'));
    await tester.pumpAndSettle();

    expect(tester.takeException(), isNull);
    expect(find.text('Focusing'), findsOneWidget);
    expect(find.text('1 day'), findsOneWidget);
  });

  testWidgets('clamps an existing custom date to the Android picker range', (
    tester,
  ) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.android;
    addTearDown(() => debugDefaultTargetPlatformOverride = null);
    final expiresAt = DateTime.now().add(const Duration(days: 730));
    await tester.pumpWidget(
      WidgetHelpers.testable(
        overrides: [
          customEmojiListProvider.overrideWithValue(const []),
          userStatusProvider.overrideWith(() => _RecordingUserStatusNotifier()),
        ],
        child: Builder(
          builder: (context) => FilledButton(
            onPressed: () => showSetStatusSheet(
              context,
              currentStatus: UserStatus(
                text: 'Sabbatical',
                emoji: '\u{1F3DD}\u{FE0F}',
                updatedAt: 1,
                expiresAt: expiresAt.millisecondsSinceEpoch ~/ 1000,
              ),
            ),
            child: const Text('Open status editor'),
          ),
        ),
      ),
    );

    await tester.tap(find.text('Open status editor'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Until'));
    await tester.pumpAndSettle();

    final picker = tester.widget<DatePickerDialog>(
      find.byType(DatePickerDialog),
    );
    expect(picker.initialDate, picker.lastDate);
    debugDefaultTargetPlatformOverride = null;
  });

  testWidgets('custom duration reveals the native until picker', (
    tester,
  ) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
    addTearDown(() => debugDefaultTargetPlatformOverride = null);
    final statusNotifier = _RecordingUserStatusNotifier();
    await tester.pumpWidget(
      WidgetHelpers.testable(
        overrides: [
          customEmojiListProvider.overrideWithValue(const []),
          userStatusProvider.overrideWith(() => statusNotifier),
        ],
        child: Builder(
          builder: (context) => FilledButton(
            onPressed: () => showSetStatusSheet(context),
            child: const Text('Open status editor'),
          ),
        ),
      ),
    );

    await tester.tap(find.text('Open status editor'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('1 day'));
    await tester.pumpAndSettle();
    await tester.ensureVisible(find.text('Custom'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Custom'));
    await tester.pumpAndSettle();

    expect(find.byType(CupertinoDatePicker), findsOneWidget);
    await tester.tap(find.text('Done'));
    await tester.pumpAndSettle();

    expect(find.text('Custom'), findsOneWidget);
    expect(find.text('Until'), findsOneWidget);
    debugDefaultTargetPlatformOverride = null;
  });
}

class _RecordingUserStatusNotifier extends UserStatusNotifier {
  String? savedText;
  String? savedEmoji;
  DateTime? savedExpiresAt;

  @override
  Future<UserStatus?> build() async => null;

  @override
  Future<void> setStatus(
    String text,
    String emoji, {
    DateTime? expiresAt,
  }) async {
    savedText = text;
    savedEmoji = emoji;
    savedExpiresAt = expiresAt;
  }
}
