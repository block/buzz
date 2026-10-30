import 'dart:async';

import 'package:buzz/features/profile/profile_avatar_draft.dart';
import 'package:buzz/features/profile/profile_edit_page.dart';
import 'package:buzz/features/profile/profile_provider.dart';
import 'package:buzz/features/profile/profile_text_editor.dart';
import 'package:buzz/shared/profile/user_profile.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../helpers/widget_helpers.dart';

void main() {
  testWidgets('settings text editor waits for profile hydration', (
    tester,
  ) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.android;
    addTearDown(() => debugDefaultTargetPlatformOverride = null);
    final notifier = _DelayedHydrationProfileNotifier();

    await tester.pumpWidget(
      WidgetHelpers.testable(
        overrides: [profileProvider.overrideWith(() => notifier)],
        child: Builder(
          builder: (context) => TextButton(
            onPressed: () => unawaited(showProfileDisplayNameEditor(context)),
            child: const Text('Open editor'),
          ),
        ),
      ),
    );
    await tester.tap(find.text('Open editor'));
    await tester.pump();
    expect(find.byKey(const ValueKey('profile-field-input')), findsNothing);

    notifier.completeHydration();
    await tester.pumpAndSettle();
    expect(
      tester
          .widget<TextField>(find.byKey(const ValueKey('profile-field-input')))
          .controller
          ?.text,
      'Hydrated name',
    );
    debugDefaultTargetPlatformOverride = null;
  });

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
    expect(
      (calls.first.arguments
          as Map<Object?, Object?>)['allowUnchangedSubmission'],
      isFalse,
    );
    expect(
      (calls.last.arguments
          as Map<Object?, Object?>)['allowUnchangedSubmission'],
      isTrue,
    );
    debugDefaultTargetPlatformOverride = null;
  });

  testWidgets('native text editor stops retrying after a community switch', (
    tester,
  ) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
    addTearDown(() => debugDefaultTargetPlatformOverride = null);
    const channel = MethodChannel('buzz/profile_text_editor');
    var presentations = 0;
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(channel, (_) async {
          presentations++;
          return 'Old community draft';
        });
    addTearDown(
      () => TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
          .setMockMethodCallHandler(channel, null),
    );
    final notifier = _CommunityChangedProfileNotifier();

    await tester.pumpWidget(
      WidgetHelpers.testable(
        overrides: [profileProvider.overrideWith(() => notifier)],
        child: const ProfileEditPage(),
      ),
    );
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const ValueKey('profile-display-name-row')));
    await tester.pumpAndSettle();

    expect(presentations, 1);
    expect(notifier.displayNameAttempts, ['Old community draft']);
    debugDefaultTargetPlatformOverride = null;
  });

  testWidgets('native text editor rejects its first save after a switch', (
    tester,
  ) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
    addTearDown(() => debugDefaultTargetPlatformOverride = null);
    const channel = MethodChannel('buzz/profile_text_editor');
    final submission = Completer<String?>();
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(channel, (_) => submission.future);
    addTearDown(
      () => TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
          .setMockMethodCallHandler(channel, null),
    );
    final notifier = _RetryProfileNotifier();
    final config = _MutableRelayConfigNotifier();

    await tester.pumpWidget(
      WidgetHelpers.testable(
        overrides: [
          profileProvider.overrideWith(() => notifier),
          relayConfigProvider.overrideWith(() => config),
        ],
        child: Builder(
          builder: (context) => TextButton(
            onPressed: () => unawaited(showProfileDisplayNameEditor(context)),
            child: const Text('Open editor'),
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();
    final container = ProviderScope.containerOf(
      tester.element(find.byType(TextButton)),
    );
    final configSubscription = container.listen(
      relayConfigProvider,
      (_, _) {},
      fireImmediately: true,
    );
    addTearDown(configSubscription.close);
    await tester.tap(find.text('Open editor'));
    await tester.pump();

    config.update(baseUrl: 'https://second.example', nsec: 'second-identity');
    submission.complete('Old community draft');
    await tester.pumpAndSettle();

    expect(notifier.displayNameAttempts, isEmpty);
    debugDefaultTargetPlatformOverride = null;
  });

  testWidgets('native text retry stops when its owner unmounts', (
    tester,
  ) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
    addTearDown(() => debugDefaultTargetPlatformOverride = null);
    const channel = MethodChannel('buzz/profile_text_editor');
    var presentations = 0;
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(channel, (_) async {
          presentations++;
          return 'Pending draft';
        });
    addTearDown(
      () => TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
          .setMockMethodCallHandler(channel, null),
    );
    final notifier = _DeferredFailureProfileNotifier();

    await tester.pumpWidget(
      WidgetHelpers.testable(
        overrides: [profileProvider.overrideWith(() => notifier)],
        child: const ProfileEditPage(),
      ),
    );
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const ValueKey('profile-display-name-row')));
    await tester.pump();
    expect(notifier.displayNameAttempts, ['Pending draft']);

    await tester.pumpWidget(const SizedBox());
    notifier.failSave();
    await tester.pump();

    expect(presentations, 1);
    debugDefaultTargetPlatformOverride = null;
  });

  testWidgets('native text retry stops when its owner route is covered', (
    tester,
  ) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
    addTearDown(() => debugDefaultTargetPlatformOverride = null);
    const channel = MethodChannel('buzz/profile_text_editor');
    var presentations = 0;
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(channel, (_) async {
          presentations++;
          return 'Pending draft';
        });
    addTearDown(
      () => TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
          .setMockMethodCallHandler(channel, null),
    );
    final notifier = _DeferredFailureProfileNotifier();

    await tester.pumpWidget(
      WidgetHelpers.testable(
        overrides: [profileProvider.overrideWith(() => notifier)],
        child: Builder(
          builder: (context) => Column(
            children: [
              TextButton(
                onPressed: () =>
                    unawaited(showProfileDisplayNameEditor(context)),
                child: const Text('Open editor'),
              ),
              TextButton(
                onPressed: () => unawaited(
                  Navigator.of(context).push<void>(
                    MaterialPageRoute<void>(
                      builder: (_) => const Scaffold(body: Text('Theme')),
                    ),
                  ),
                ),
                child: const Text('Open destination'),
              ),
            ],
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();
    await tester.tap(find.text('Open editor'));
    await tester.pump();
    expect(notifier.displayNameAttempts, ['Pending draft']);

    await tester.tap(find.text('Open destination'));
    await tester.pumpAndSettle();
    notifier.failSave();
    await tester.pump();

    expect(presentations, 1);
    debugDefaultTargetPlatformOverride = null;
  });

  testWidgets('in-page text editor rejects its first save after a switch', (
    tester,
  ) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.android;
    addTearDown(() => debugDefaultTargetPlatformOverride = null);
    final notifier = _RetryProfileNotifier();
    final config = _MutableRelayConfigNotifier();

    await tester.pumpWidget(
      WidgetHelpers.testable(
        overrides: [
          profileProvider.overrideWith(() => notifier),
          relayConfigProvider.overrideWith(() => config),
        ],
        child: const ProfileEditPage(),
      ),
    );
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const ValueKey('profile-display-name-row')));
    await tester.pumpAndSettle();
    await tester.enterText(
      find.byKey(const ValueKey('profile-field-input')),
      'Old community draft',
    );
    await tester.pump();
    config.update(baseUrl: 'https://second.example', nsec: 'second-identity');
    await tester.tap(find.byKey(const ValueKey('profile-field-save')));
    await tester.pumpAndSettle();

    expect(find.byKey(const ValueKey('profile-field-input')), findsNothing);
    expect(notifier.displayNameAttempts, isEmpty);
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

  testWidgets('avatar draft cannot save after a prior community switch', (
    tester,
  ) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.android;
    addTearDown(() => debugDefaultTargetPlatformOverride = null);
    final notifier = _RetryProfileNotifier();
    final config = _MutableRelayConfigNotifier();
    final firstUpload = _RetryMediaUploadService(
      baseUrl: 'https://first.example',
    );
    final secondUpload = _RetryMediaUploadService(
      baseUrl: 'https://second.example',
    );
    addTearDown(firstUpload.dispose);
    addTearDown(secondUpload.dispose);

    await tester.pumpWidget(
      WidgetHelpers.testable(
        overrides: [
          profileProvider.overrideWith(() => notifier),
          relayConfigProvider.overrideWith(() => config),
          mediaUploadServiceProvider.overrideWith((ref) {
            final current = ref.watch(relayConfigProvider);
            return current.baseUrl == 'https://first.example'
                ? firstUpload
                : secondUpload;
          }),
        ],
        child: ProfileEditPage(
          startInPhotoEditor: true,
          animatedAvatarCaptureBuilder:
              ({required height, required onPrepareChanged}) => HookBuilder(
                builder: (context) {
                  useEffect(() {
                    WidgetsBinding.instance.addPostFrameCallback((_) {
                      onPrepareChanged(
                        () async => ProfileImageAvatarDraft(
                          Uint8List.fromList([1, 2, 3]),
                        ),
                      );
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

    config.update(baseUrl: 'https://second.example', nsec: 'second-identity');
    await tester.tap(find.byKey(const ValueKey('avatar-save')));
    await tester.pumpAndSettle();

    expect(firstUpload.uploadCount, 0);
    expect(secondUpload.uploadCount, 0);
    expect(notifier.savedAvatarUrls, isEmpty);
    expect(find.byKey(const ValueKey('avatar-save')), findsNothing);
    debugDefaultTargetPlatformOverride = null;
  });

  testWidgets('avatar editor closes when community changes during save', (
    tester,
  ) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.android;
    addTearDown(() => debugDefaultTargetPlatformOverride = null);
    final notifier = _RetryProfileNotifier();
    final config = _MutableRelayConfigNotifier();
    final firstUpload = _RetryMediaUploadService(
      baseUrl: 'https://first.example',
      delayUpload: true,
    );
    final secondUpload = _RetryMediaUploadService(
      baseUrl: 'https://second.example',
    );
    addTearDown(firstUpload.dispose);
    addTearDown(secondUpload.dispose);

    await tester.pumpWidget(
      WidgetHelpers.testable(
        overrides: [
          profileProvider.overrideWith(() => notifier),
          relayConfigProvider.overrideWith(() => config),
          mediaUploadServiceProvider.overrideWith((ref) {
            final current = ref.watch(relayConfigProvider);
            return current.baseUrl == 'https://first.example'
                ? firstUpload
                : secondUpload;
          }),
        ],
        child: ProfileEditPage(
          startInPhotoEditor: true,
          animatedAvatarCaptureBuilder:
              ({required height, required onPrepareChanged}) => HookBuilder(
                builder: (context) {
                  useEffect(() {
                    WidgetsBinding.instance.addPostFrameCallback((_) {
                      onPrepareChanged(
                        () async => ProfileImageAvatarDraft(
                          Uint8List.fromList([1, 2, 3]),
                        ),
                      );
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
    await tester.pump();
    expect(firstUpload.uploadCount, 1);
    config.update(baseUrl: 'https://second.example', nsec: 'second-identity');
    firstUpload.completeUpload();
    await tester.pumpAndSettle();

    expect(notifier.savedAvatarUrls, isEmpty);
    expect(secondUpload.uploadCount, 0);
    expect(find.byKey(const ValueKey('avatar-save')), findsNothing);
    debugDefaultTargetPlatformOverride = null;
  });

  testWidgets('avatar editor closes when a switched upload throws', (
    tester,
  ) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.android;
    addTearDown(() => debugDefaultTargetPlatformOverride = null);
    final notifier = _RetryProfileNotifier();
    final config = _MutableRelayConfigNotifier();
    final firstUpload = _RetryMediaUploadService(
      baseUrl: 'https://first.example',
      delayUpload: true,
    );
    final secondUpload = _RetryMediaUploadService(
      baseUrl: 'https://second.example',
    );
    addTearDown(firstUpload.dispose);
    addTearDown(secondUpload.dispose);

    await tester.pumpWidget(
      WidgetHelpers.testable(
        overrides: [
          profileProvider.overrideWith(() => notifier),
          relayConfigProvider.overrideWith(() => config),
          mediaUploadServiceProvider.overrideWith((ref) {
            final current = ref.watch(relayConfigProvider);
            return current.baseUrl == 'https://first.example'
                ? firstUpload
                : secondUpload;
          }),
        ],
        child: ProfileEditPage(
          startInPhotoEditor: true,
          animatedAvatarCaptureBuilder:
              ({required height, required onPrepareChanged}) => HookBuilder(
                builder: (context) {
                  useEffect(() {
                    WidgetsBinding.instance.addPostFrameCallback((_) {
                      onPrepareChanged(
                        () async => ProfileImageAvatarDraft(
                          Uint8List.fromList([1, 2, 3]),
                        ),
                      );
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
    await tester.pump();
    config.update(baseUrl: 'https://second.example', nsec: 'second-identity');
    firstUpload.failUpload();
    await tester.pumpAndSettle();

    expect(notifier.savedAvatarUrls, isEmpty);
    expect(find.byKey(const ValueKey('avatar-save')), findsNothing);
    debugDefaultTargetPlatformOverride = null;
  });
}

class _MutableRelayConfigNotifier extends RelayConfigNotifier {
  @override
  RelayConfig build() => const RelayConfig(
    baseUrl: 'https://first.example',
    nsec: 'first-identity',
  );
}

class _DelayedHydrationProfileNotifier extends ProfileNotifier {
  final _hydration = Completer<UserProfile?>();

  @override
  Future<UserProfile?> build() => _hydration.future;

  void completeHydration() => _hydration.complete(
    const UserProfile(pubkey: 'aabb', displayName: 'Hydrated name'),
  );
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

class _CommunityChangedProfileNotifier extends ProfileNotifier {
  final displayNameAttempts = <String>[];

  @override
  Future<UserProfile?> build() async => const UserProfile(
    pubkey: 'aabb',
    displayName: 'Alice',
    about: 'Building Buzz',
  );

  @override
  Future<void> updateDisplayName(String displayName) async {
    displayNameAttempts.add(displayName);
    throw ProfileCommunityChangedException();
  }
}

class _DeferredFailureProfileNotifier extends ProfileNotifier {
  final displayNameAttempts = <String>[];
  final _save = Completer<void>();

  @override
  Future<UserProfile?> build() async => const UserProfile(
    pubkey: 'aabb',
    displayName: 'Alice',
    about: 'Building Buzz',
  );

  @override
  Future<void> updateDisplayName(String displayName) {
    displayNameAttempts.add(displayName);
    return _save.future;
  }

  void failSave() => _save.completeError(Exception('profile publish failed'));
}

class _RetryMediaUploadService extends MediaUploadService {
  _RetryMediaUploadService({
    this.baseUrl = 'https://relay.example',
    this.delayUpload = false,
  }) : super(
         baseUrl: baseUrl,
         nsec: null,
         pickGalleryImage: () async => null,
         pickGalleryVideo: () async => null,
       );

  final String baseUrl;
  final bool delayUpload;
  final _pendingUpload = Completer<void>();
  int uploadCount = 0;

  void completeUpload() {
    if (!_pendingUpload.isCompleted) _pendingUpload.complete();
  }

  void failUpload() {
    if (!_pendingUpload.isCompleted) {
      _pendingUpload.completeError(Exception('upload client closed'));
    }
  }

  @override
  Future<BlobDescriptor> uploadBytes(
    Uint8List bytes, {
    required String mimeType,
    ValueChanged<double>? onProgress,
    UploadCancellationToken? cancellationToken,
  }) async {
    uploadCount++;
    if (delayUpload) await _pendingUpload.future;
    return BlobDescriptor(
      url: '$baseUrl/avatar.jpg',
      sha256: 'avatar-hash',
      size: bytes.length,
      type: mimeType,
      uploaded: 1,
    );
  }
}
