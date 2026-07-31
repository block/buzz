import 'dart:convert';

import 'package:buzz/features/profile/edit_profile_sheet.dart';
import 'package:buzz/features/profile/profile_provider.dart';
import 'package:buzz/features/profile/settings_profile_header.dart';
import 'package:buzz/features/profile/user_profile.dart';
import 'package:buzz/features/profile/user_status.dart';
import 'package:buzz/features/profile/user_status_provider.dart';
import 'package:buzz/shared/custom_emoji/custom_emoji_provider.dart';
import 'package:buzz/shared/widgets/masked_avatar_badge.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';

import '../../helpers/widget_helpers.dart';

void main() {
  test('builds a self-contained escaped emoji SVG avatar', () {
    final avatar = emojiAvatarDataUrl('🐝<&', '#F4B942');
    final svg = utf8.decode(UriData.parse(avatar).contentAsBytes());

    expect(avatar, startsWith('data:image/svg+xml,'));
    expect(svg, contains('<rect'));
    expect(svg, contains('fill="#F4B942"'));
    expect(svg, contains('🐝&lt;&amp;'));
  });

  testWidgets('uses a bounded icon for an unresolved status shortcode', (
    tester,
  ) async {
    const missingShortcode = ':very_long_missing_custom_emoji:';
    await tester.pumpWidget(
      WidgetHelpers.testable(
        overrides: [
          profileProvider.overrideWith(_FakeProfileNotifier.new),
          userStatusProvider.overrideWith(
            () => _FakeUserStatusNotifier(
              const UserStatus(
                text: 'Focusing',
                emoji: missingShortcode,
                updatedAt: 1,
              ),
            ),
          ),
          customEmojiListProvider.overrideWithValue(const []),
        ],
        child: const SettingsProfileHeader(),
      ),
    );
    await tester.pumpAndSettle();

    final badge = find.byType(MaskedAvatarBadge);
    expect(
      find.descendant(of: badge, matching: find.text(missingShortcode)),
      findsNothing,
    );
    expect(
      find.descendant(of: badge, matching: find.byIcon(LucideIcons.smile)),
      findsOneWidget,
    );
  });

  testWidgets('edits and saves the current profile', (tester) async {
    final profile = _SavingProfileNotifier();
    await tester.pumpWidget(
      WidgetHelpers.testable(
        overrides: [
          profileProvider.overrideWith(() => profile),
          userStatusProvider.overrideWith(() => _FakeUserStatusNotifier(null)),
        ],
        child: const SettingsProfileHeader(),
      ),
    );
    await tester.pumpAndSettle();

    await tester.tap(find.widgetWithText(TextButton, 'Edit profile'));
    await tester.pumpAndSettle();
    expect(find.text('Edit profile'), findsWidgets);

    await tester.enterText(find.byType(TextField).at(0), 'New Name');
    await tester.enterText(find.byType(TextField).at(1), '🦊');
    await tester.tap(find.widgetWithText(FilledButton, 'Save profile'));
    await tester.pumpAndSettle();

    expect(profile.savedDisplayName, 'New Name');
    expect(profile.savedAvatarUrl, startsWith('data:image/svg+xml,'));
    expect(find.text('Profile saved'), findsOneWidget);
    expect(find.widgetWithText(FilledButton, 'Save profile'), findsNothing);
  });

  testWidgets('keeps the editor open and surfaces save failures', (
    tester,
  ) async {
    final profile = _SavingProfileNotifier(
      error: Exception('relay rejected profile'),
    );
    await tester.pumpWidget(
      WidgetHelpers.testable(
        overrides: [
          profileProvider.overrideWith(() => profile),
          userStatusProvider.overrideWith(() => _FakeUserStatusNotifier(null)),
        ],
        child: const SettingsProfileHeader(),
      ),
    );
    await tester.pumpAndSettle();

    await tester.tap(find.widgetWithText(TextButton, 'Edit profile'));
    await tester.pumpAndSettle();
    await tester.enterText(find.byType(TextField).at(0), 'Unsaved Name');
    await tester.tap(find.widgetWithText(FilledButton, 'Save profile'));
    await tester.pumpAndSettle();

    expect(find.text('Unable to save profile'), findsOneWidget);
    expect(find.widgetWithText(FilledButton, 'Save profile'), findsOneWidget);
  });
}

class _FakeProfileNotifier extends ProfileNotifier {
  @override
  Future<UserProfile?> build() async =>
      const UserProfile(pubkey: 'aabb', displayName: 'Test');
}

class _FakeUserStatusNotifier extends UserStatusNotifier {
  _FakeUserStatusNotifier(this._status);

  final UserStatus? _status;

  @override
  Future<UserStatus?> build() async => _status;
}

class _SavingProfileNotifier extends ProfileNotifier {
  _SavingProfileNotifier({this.error});

  final Object? error;
  String? savedDisplayName;
  String? savedAvatarUrl;

  @override
  Future<UserProfile?> build() async =>
      const UserProfile(pubkey: 'aabb', displayName: 'Old Name');

  @override
  Future<UserProfile> saveProfile({
    required String displayName,
    String? avatarUrl,
  }) async {
    savedDisplayName = displayName;
    savedAvatarUrl = avatarUrl;
    if (error case final failure?) throw failure;
    return UserProfile(
      pubkey: 'aabb',
      displayName: displayName,
      avatarUrl: avatarUrl,
    );
  }
}
