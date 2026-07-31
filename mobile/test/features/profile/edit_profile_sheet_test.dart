import 'package:buzz/features/profile/edit_profile_sheet.dart';
import 'package:buzz/features/profile/profile_provider.dart';
import 'package:buzz/features/profile/settings_profile_header.dart';
import 'package:buzz/features/profile/user_profile.dart';
import 'package:buzz/features/profile/user_status.dart';
import 'package:buzz/features/profile/user_status_provider.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import '../../helpers/widget_helpers.dart';

void main() {
  testWidgets('profile-less identity saves name, uploaded avatar, and bio', (
    tester,
  ) async {
    final profileNotifier = _RecordingProfileNotifier();
    await tester.pumpWidget(
      WidgetHelpers.testable(
        overrides: [
          profileProvider.overrideWith(() => profileNotifier),
          userStatusProvider.overrideWith(_EmptyStatusNotifier.new),
          profileAvatarPickerProvider.overrideWithValue(
            () async => 'https://example.com/avatar.png',
          ),
        ],
        child: const SettingsProfileHeader(),
      ),
    );
    await tester.pumpAndSettle();

    expect(find.text('Your profile'), findsOneWidget);
    await tester.tap(find.text('Edit profile'));
    await tester.pumpAndSettle();

    await tester.enterText(
      find.widgetWithText(TextField, 'How others see you'),
      'Android Alice',
    );
    await tester.enterText(
      find.widgetWithText(TextField, 'A little about you'),
      'Testing mobile Buzz',
    );
    await tester.tap(find.text('Choose photo'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Save'));
    await tester.pumpAndSettle();

    expect(profileNotifier.savedDisplayName, 'Android Alice');
    expect(profileNotifier.savedAvatarUrl, 'https://example.com/avatar.png');
    expect(profileNotifier.savedAbout, 'Testing mobile Buzz');
    expect(find.text('Edit profile'), findsOneWidget);
    expect(find.text('Profile saved'), findsOneWidget);
  });

  testWidgets('failed save stays open and shows stable error copy', (
    tester,
  ) async {
    final profileNotifier = _RecordingProfileNotifier(shouldFail: true);
    await tester.pumpWidget(
      WidgetHelpers.testable(
        overrides: [
          profileProvider.overrideWith(() => profileNotifier),
          userStatusProvider.overrideWith(_EmptyStatusNotifier.new),
        ],
        child: const SettingsProfileHeader(),
      ),
    );
    await tester.pumpAndSettle();

    await tester.tap(find.text('Edit profile'));
    await tester.pumpAndSettle();
    await tester.enterText(
      find.widgetWithText(TextField, 'How others see you'),
      'Unsaved',
    );
    await tester.pump();
    await tester.tap(find.text('Save'));
    await tester.pumpAndSettle();

    expect(profileNotifier.updateCalls, 1);
    expect(find.text('Edit profile'), findsWidgets);
    expect(find.text('Couldn\u2019t save your profile. Try again.'), findsOne);
    expect(find.text('Profile saved'), findsNothing);
  });
}

class _RecordingProfileNotifier extends ProfileNotifier {
  _RecordingProfileNotifier({this.shouldFail = false});

  final bool shouldFail;
  String? savedDisplayName;
  String? savedAvatarUrl;
  String? savedAbout;
  int updateCalls = 0;

  @override
  Future<UserProfile?> build() async => null;

  @override
  Future<UserProfile> updateProfile({
    required String displayName,
    required String avatarUrl,
    required String about,
  }) async {
    updateCalls += 1;
    if (shouldFail) throw Exception('raw relay failure');
    savedDisplayName = displayName;
    savedAvatarUrl = avatarUrl;
    savedAbout = about;
    return UserProfile(
      pubkey: 'aabb',
      displayName: displayName,
      avatarUrl: avatarUrl,
      about: about,
    );
  }
}

class _EmptyStatusNotifier extends UserStatusNotifier {
  @override
  Future<UserStatus?> build() async => null;
}
