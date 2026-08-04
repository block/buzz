import 'dart:async';

import 'package:buzz/features/profile/profile_provider.dart';
import 'package:buzz/features/profile/settings_profile_header.dart';
import 'package:buzz/features/profile/user_profile.dart';
import 'package:buzz/features/profile/user_status_provider.dart';
import 'package:buzz/shared/custom_emoji/custom_emoji_provider.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import '../../helpers/widget_helpers.dart';

void main() {
  testWidgets('opens the display-name editor with the current value', (
    tester,
  ) async {
    await tester.pumpWidget(
      WidgetHelpers.testable(
        overrides: [
          profileProvider.overrideWith(_FakeProfileNotifier.new),
          userStatusProvider.overrideWith(_FakeUserStatusNotifier.new),
          customEmojiListProvider.overrideWithValue(const []),
        ],
        child: const SettingsProfileHeader(),
      ),
    );
    await tester.pumpAndSettle();

    expect(find.text('Daniel'), findsOneWidget);
    expect(find.byTooltip('Edit profile'), findsOneWidget);

    await tester.tap(find.byTooltip('Edit profile'));
    await tester.pumpAndSettle();

    expect(find.text('Edit profile'), findsOneWidget);
    expect(
      tester.widget<TextField>(find.byType(TextField)).controller?.text,
      'Daniel',
    );
    await tester.tap(find.byType(TextField));
    tester.testTextInput.enterText('x' * 256);
    await tester.pump();
    expect(
      tester.widget<TextField>(find.byType(TextField)).controller?.text.length,
      255,
    );
  });

  testWidgets('saves a changed display name and closes the editor', (
    tester,
  ) async {
    final profile = _RecordingProfileNotifier();
    await tester.pumpWidget(
      WidgetHelpers.testable(
        overrides: [
          profileProvider.overrideWith(() => profile),
          userStatusProvider.overrideWith(_FakeUserStatusNotifier.new),
          customEmojiListProvider.overrideWithValue(const []),
        ],
        child: const SettingsProfileHeader(),
      ),
    );
    await tester.pumpAndSettle();

    await tester.tap(find.byTooltip('Edit profile'));
    await tester.pumpAndSettle();
    await tester.enterText(find.byType(TextField), 'Daniel V');
    await tester.pump();
    await tester.tap(find.widgetWithText(FilledButton, 'Save'));
    await tester.pumpAndSettle();

    expect(profile.savedDisplayName, 'Daniel V');
    expect(find.text('Edit profile'), findsNothing);
  });

  testWidgets('keeps the draft visible when publishing fails', (tester) async {
    final profile = _RecordingProfileNotifier(shouldFail: true);
    await tester.pumpWidget(
      WidgetHelpers.testable(
        overrides: [
          profileProvider.overrideWith(() => profile),
          userStatusProvider.overrideWith(_FakeUserStatusNotifier.new),
          customEmojiListProvider.overrideWithValue(const []),
        ],
        child: const SettingsProfileHeader(),
      ),
    );
    await tester.pumpAndSettle();

    await tester.tap(find.byTooltip('Edit profile'));
    await tester.pumpAndSettle();
    await tester.enterText(find.byType(TextField), 'Unsaved name');
    await tester.pump();
    await tester.tap(find.widgetWithText(FilledButton, 'Save'));
    await tester.pumpAndSettle();

    expect(find.text('Could not update your profile. Try again.'), findsOne);
    expect(
      tester.widget<TextField>(find.byType(TextField)).controller?.text,
      'Unsaved name',
    );
  });

  testWidgets('keeps a long display name within the settings header', (
    tester,
  ) async {
    await tester.pumpWidget(
      WidgetHelpers.testable(
        overrides: [
          profileProvider.overrideWith(_LongNameProfileNotifier.new),
          userStatusProvider.overrideWith(_FakeUserStatusNotifier.new),
          customEmojiListProvider.overrideWithValue(const []),
        ],
        child: const SettingsProfileHeader(),
      ),
    );
    await tester.pumpAndSettle();

    expect(tester.takeException(), isNull);
    expect(find.byTooltip('Edit profile'), findsOneWidget);
  });

  testWidgets('can be dismissed while a profile update is pending', (
    tester,
  ) async {
    final profile = _PendingProfileNotifier();
    await tester.pumpWidget(
      WidgetHelpers.testable(
        overrides: [
          profileProvider.overrideWith(() => profile),
          userStatusProvider.overrideWith(_FakeUserStatusNotifier.new),
          customEmojiListProvider.overrideWithValue(const []),
        ],
        child: const SettingsProfileHeader(),
      ),
    );
    await tester.pumpAndSettle();

    await tester.tap(find.byTooltip('Edit profile'));
    await tester.pumpAndSettle();
    await tester.enterText(find.byType(TextField), 'Daniel V');
    await tester.pump();
    await tester.tap(find.widgetWithText(FilledButton, 'Save'));
    await tester.pump();
    await tester.tapAt(const Offset(10, 10));
    await tester.pumpAndSettle();

    profile.save.completeError(Exception('relay unavailable'));
    await tester.pump();

    expect(find.text('Edit profile'), findsNothing);
    expect(tester.takeException(), isNull);
  });
}

class _FakeProfileNotifier extends ProfileNotifier {
  @override
  Future<UserProfile?> build() async =>
      const UserProfile(pubkey: 'aabb', displayName: 'Daniel');
}

class _RecordingProfileNotifier extends _FakeProfileNotifier {
  _RecordingProfileNotifier({this.shouldFail = false});

  final bool shouldFail;
  String? savedDisplayName;

  @override
  Future<void> updateDisplayName(String displayName) async {
    if (shouldFail) throw Exception('relay unavailable');
    savedDisplayName = displayName;
  }
}

class _LongNameProfileNotifier extends ProfileNotifier {
  @override
  Future<UserProfile?> build() async => const UserProfile(
    pubkey: 'aabb',
    displayName:
        'A display name long enough to exceed the available settings width',
  );
}

class _PendingProfileNotifier extends _FakeProfileNotifier {
  final save = Completer<void>();

  @override
  Future<void> updateDisplayName(String displayName) => save.future;
}

class _FakeUserStatusNotifier extends UserStatusNotifier {
  @override
  Future<Never?> build() async => null;
}
