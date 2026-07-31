import 'dart:typed_data';

import 'package:buzz/features/profile/edit_profile_sheet.dart';
import 'package:buzz/features/profile/profile_provider.dart';
import 'package:buzz/features/profile/settings_profile_header.dart';
import 'package:buzz/features/profile/user_profile.dart';
import 'package:buzz/features/profile/user_status.dart';
import 'package:buzz/features/profile/user_status_provider.dart';
import 'package:buzz/shared/custom_emoji/custom_emoji.dart';
import 'package:buzz/shared/custom_emoji/custom_emoji_provider.dart';
import 'package:buzz/shared/emoji/emoji_data.dart';
import 'package:buzz/shared/emoji/emoji_data_provider.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/theme_provider.dart';
import 'package:buzz/shared/widgets/avatar_image.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:image_picker/image_picker.dart';
import 'package:shared_preferences/shared_preferences.dart';

import '../../helpers/widget_helpers.dart';

void main() {
  test('profile avatar selection is bounded before JPEG upload', () async {
    double? capturedMaxWidth;
    double? capturedMaxHeight;
    int? capturedImageQuality;
    final uploadService = _RecordingMediaUploadService();
    final container = ProviderContainer(
      overrides: [
        profileAvatarImagePickerProvider.overrideWithValue(({
          required maxWidth,
          required maxHeight,
          required imageQuality,
        }) async {
          capturedMaxWidth = maxWidth;
          capturedMaxHeight = maxHeight;
          capturedImageQuality = imageQuality;
          return XFile.fromData(
            Uint8List.fromList([1, 2, 3]),
            name: 'avatar.png',
          );
        }),
        mediaUploadServiceProvider.overrideWithValue(uploadService),
      ],
    );
    addTearDown(container.dispose);

    final url = await container.read(profileAvatarPickerProvider)();

    expect(capturedMaxWidth, 512);
    expect(capturedMaxHeight, 512);
    expect(capturedImageQuality, 85);
    expect(await uploadService.uploadedImage?.readAsBytes(), [1, 2, 3]);
    expect(url, 'https://example.com/avatar.jpg');
  });

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

  testWidgets('selects a standard emoji and saves it with the profile', (
    tester,
  ) async {
    final profileNotifier = _RecordingProfileNotifier();
    await _pumpProfileHeader(tester, profileNotifier: profileNotifier);

    await tester.tap(find.text('Edit profile'));
    await tester.pumpAndSettle();
    await tester.enterText(
      find.widgetWithText(TextField, 'How others see you'),
      'Emoji Alice',
    );
    await tester.tap(find.text('Use emoji'));
    await tester.pumpAndSettle();

    expect(
      find.byKey(const ValueKey('emoji-tile-custom-partyparrot')),
      findsNothing,
    );
    await tester.tap(find.byKey(const ValueKey('emoji-tile-bee')));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const ValueKey('avatar-color-#E66A6A')));
    await tester.tap(find.text('Save'));
    await tester.pumpAndSettle();

    final avatar = parseEmojiAvatarDataUrl(profileNotifier.savedAvatarUrl);
    expect(avatar?.emoji, '🐝');
    expect(avatar?.color, '#E66A6A');
  });

  testWidgets('restores an existing emoji and can replace it with a photo', (
    tester,
  ) async {
    final profileNotifier = _RecordingProfileNotifier(
      currentProfile: UserProfile(
        pubkey: 'aabb',
        displayName: 'Emoji Alice',
        avatarUrl: emojiAvatarDataUrl('🦊', '#5B8DEF'),
        about: 'Existing bio',
      ),
    );
    await _pumpProfileHeader(
      tester,
      profileNotifier: profileNotifier,
      pickedPhotoUrl: 'https://example.com/replacement.jpg',
    );

    await tester.tap(find.text('Edit profile'));
    await tester.pumpAndSettle();

    expect(find.text('🦊'), findsWidgets);
    expect(find.bySemanticsLabel('Emoji avatar color'), findsOneWidget);
    await tester.tap(find.text('Choose photo'));
    await tester.pumpAndSettle();
    expect(find.bySemanticsLabel('Emoji avatar color'), findsNothing);
    await tester.tap(find.text('Save'));
    await tester.pumpAndSettle();

    expect(
      profileNotifier.savedAvatarUrl,
      'https://example.com/replacement.jpg',
    );
    expect(profileNotifier.savedAbout, 'Existing bio');
  });

  testWidgets('removes a restored emoji avatar', (tester) async {
    final profileNotifier = _RecordingProfileNotifier(
      currentProfile: UserProfile(
        pubkey: 'aabb',
        displayName: 'Emoji Alice',
        avatarUrl: emojiAvatarDataUrl('🦊', '#5B8DEF'),
      ),
    );
    await _pumpProfileHeader(tester, profileNotifier: profileNotifier);

    await tester.tap(find.text('Edit profile'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Remove'));
    await tester.pumpAndSettle();

    expect(find.bySemanticsLabel('Emoji avatar color'), findsNothing);
    await tester.tap(find.text('Save'));
    await tester.pumpAndSettle();
    expect(profileNotifier.savedAvatarUrl, isEmpty);
  });
}

final _emojiDataset = EmojiDataset(
  categories: const [
    EmojiCategory(
      id: 'people',
      emoji: [
        EmojiEntry(
          id: 'bee',
          name: 'Bee',
          keywords: ['insect'],
          native: '🐝',
          categoryId: 'people',
        ),
      ],
    ),
  ],
  all: const [
    EmojiEntry(
      id: 'bee',
      name: 'Bee',
      keywords: ['insect'],
      native: '🐝',
      categoryId: 'people',
    ),
  ],
  nativeToShortcode: const {'🐝': ':bee:'},
);

Future<void> _pumpProfileHeader(
  WidgetTester tester, {
  required _RecordingProfileNotifier profileNotifier,
  String? pickedPhotoUrl,
}) async {
  SharedPreferences.setMockInitialValues({});
  final prefs = await SharedPreferences.getInstance();
  await tester.pumpWidget(
    WidgetHelpers.testable(
      overrides: [
        profileProvider.overrideWith(() => profileNotifier),
        userStatusProvider.overrideWith(_EmptyStatusNotifier.new),
        profileAvatarPickerProvider.overrideWithValue(
          () async => pickedPhotoUrl,
        ),
        savedPrefsProvider.overrideWithValue(prefs),
        emojiDatasetOrEmptyProvider.overrideWithValue(_emojiDataset),
        customEmojiListProvider.overrideWithValue(const [
          CustomEmoji(
            shortcode: 'partyparrot',
            url: 'https://example.test/parrot.gif',
          ),
        ]),
      ],
      child: const SettingsProfileHeader(),
    ),
  );
  await tester.pumpAndSettle();
}

class _RecordingMediaUploadService extends MediaUploadService {
  _RecordingMediaUploadService()
    : super(
        baseUrl: 'https://relay.example',
        nsec: null,
        pickGalleryImage: () async => null,
        pickGalleryVideo: () async => null,
      );

  XFile? uploadedImage;

  @override
  Future<BlobDescriptor> uploadProfileImage(XFile image) async {
    uploadedImage = image;
    return const BlobDescriptor(
      url: 'https://example.com/avatar.jpg',
      sha256: 'sha256',
      size: 3,
      type: 'image/jpeg',
      uploaded: 1,
    );
  }
}

class _RecordingProfileNotifier extends ProfileNotifier {
  _RecordingProfileNotifier({this.shouldFail = false, this.currentProfile});

  final bool shouldFail;
  final UserProfile? currentProfile;
  String? savedDisplayName;
  String? savedAvatarUrl;
  String? savedAbout;
  int updateCalls = 0;

  @override
  Future<UserProfile?> build() async => currentProfile;

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
