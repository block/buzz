import 'package:buzz/features/profile/profile_avatar_draft.dart';
import 'package:buzz/features/profile/profile_edit_page.dart';
import 'package:buzz/features/profile/profile_provider.dart';
import 'package:buzz/shared/profile/user_profile.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:flutter_test/flutter_test.dart';

import '../../helpers/widget_helpers.dart';

void main() {
  testWidgets('native text retry retains the failed value', (tester) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
    addTearDown(() => debugDefaultTargetPlatformOverride = null);
    const channel = MethodChannel('buzz/profile_text_editor');
    final calls = <MethodCall>[];
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(channel, (call) async {
          calls.add(call);
          return calls.length == 1 ? 'Alice Retained' : null;
        });
    addTearDown(
      () => TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
          .setMockMethodCallHandler(channel, null),
    );
    final notifier = _RetryProfileNotifier(failedTextSaves: 1);

    await tester.pumpWidget(
      WidgetHelpers.testable(
        overrides: [profileProvider.overrideWith(() => notifier)],
        child: const ProfileEditPage(),
      ),
    );
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const ValueKey('profile-display-name-row')));
    await tester.pumpAndSettle();

    expect(notifier.displayNameAttempts, ['Alice Retained']);
    expect(calls, hasLength(2));
    expect(
      (calls.last.arguments as Map<Object?, Object?>)['initialValue'],
      'Alice Retained',
    );
    debugDefaultTargetPlatformOverride = null;
  });

  testWidgets('animated save retry reuses its prepared draft', (tester) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.android;
    addTearDown(() => debugDefaultTargetPlatformOverride = null);
    final notifier = _RetryProfileNotifier(failedAvatarSaves: 1);
    final uploadService = _RetryMediaUploadService();
    addTearDown(uploadService.dispose);
    var prepareCalls = 0;

    await tester.pumpWidget(
      WidgetHelpers.testable(
        overrides: [
          profileProvider.overrideWith(() => notifier),
          mediaUploadServiceProvider.overrideWithValue(uploadService),
        ],
        child: ProfileEditPage(
          startInPhotoEditor: true,
          animatedAvatarCaptureBuilder:
              ({required height, required onPrepareChanged}) => HookBuilder(
                builder: (context) {
                  useEffect(() {
                    WidgetsBinding.instance.addPostFrameCallback((_) {
                      onPrepareChanged(() async {
                        prepareCalls++;
                        return ProfileImageAvatarDraft(
                          Uint8List.fromList([1, 2, 3]),
                        );
                      });
                    });
                    return null;
                  }, const []);
                  return SizedBox(height: height);
                },
              ),
        ),
      ),
    );
    await tester.pumpAndSettle();
    await tester.tap(find.text('Animated'));
    await tester.pumpAndSettle();

    await tester.tap(find.byKey(const ValueKey('avatar-save')));
    await tester.pumpAndSettle();
    expect(
      find.text("We couldn't save your profile photo. Try again."),
      findsOneWidget,
    );
    expect(prepareCalls, 1);
    expect(uploadService.uploadCount, 1);

    await tester.tap(find.byKey(const ValueKey('avatar-save')));
    await tester.pumpAndSettle();
    expect(prepareCalls, 1);
    expect(uploadService.uploadCount, 1);
    expect(notifier.savedAvatarUrls, ['https://relay.example/avatar.jpg']);
    debugDefaultTargetPlatformOverride = null;
  });
}

class _RetryProfileNotifier extends ProfileNotifier {
  _RetryProfileNotifier({this.failedTextSaves = 0, this.failedAvatarSaves = 0});

  int failedTextSaves;
  int failedAvatarSaves;
  final displayNameAttempts = <String>[];
  final savedAvatarUrls = <String>[];

  @override
  Future<UserProfile?> build() async => const UserProfile(
    pubkey: 'aabb',
    displayName: 'Alice',
    about: 'Building Buzz',
  );

  @override
  Future<void> updateDisplayName(String displayName) async {
    displayNameAttempts.add(displayName);
    if (failedTextSaves > 0) {
      failedTextSaves--;
      throw Exception('profile publish failed');
    }
  }

  @override
  Future<void> updateAvatarUrl(String avatarUrl) async {
    if (failedAvatarSaves > 0) {
      failedAvatarSaves--;
      throw Exception('profile publish failed');
    }
    savedAvatarUrls.add(avatarUrl);
  }
}

class _RetryMediaUploadService extends MediaUploadService {
  _RetryMediaUploadService()
    : super(
        baseUrl: 'https://relay.example',
        nsec: null,
        pickGalleryImage: () async => null,
        pickGalleryVideo: () async => null,
      );

  int uploadCount = 0;

  @override
  Future<BlobDescriptor> uploadBytes(
    Uint8List bytes, {
    required String mimeType,
    ValueChanged<double>? onProgress,
    UploadCancellationToken? cancellationToken,
  }) async {
    uploadCount++;
    return BlobDescriptor(
      url: 'https://relay.example/avatar.jpg',
      sha256: 'avatar-hash',
      size: bytes.length,
      type: mimeType,
      uploaded: 1,
    );
  }
}
