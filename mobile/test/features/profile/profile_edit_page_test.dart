import 'dart:async';
import 'dart:math' as math;

import 'package:buzz/features/profile/profile_edit_page.dart';
import 'package:buzz/features/profile/profile_avatar_draft.dart';
import 'package:buzz/features/profile/profile_avatar_crop_page.dart';
import 'package:buzz/features/profile/avatar_background_grid.dart';
import 'package:buzz/features/profile/avatar_editor_option_button.dart';
import 'package:buzz/features/profile/animated_avatar_capture.dart';
import 'package:buzz/features/profile/emoji_avatar_tile.dart';
import 'package:buzz/features/profile/image_avatar_capture.dart';
import 'package:buzz/shared/widgets/immediate_page_route.dart';
import 'package:buzz/features/profile/profile_provider.dart';
import 'package:buzz/shared/emoji/emoji_avatar.dart';
import 'package:buzz/shared/emoji/emoji_data.dart';
import 'package:buzz/shared/emoji/emoji_data_provider.dart';
import 'package:buzz/shared/emoji/native_emoji_glyph.dart';
import 'package:buzz/shared/profile/user_profile.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:buzz/shared/widgets/avatar_image.dart';
import 'package:buzz/shared/widgets/frosted_app_bar.dart';
import 'package:buzz/shared/widgets/ios_native_segmented_control.dart';
import 'package:buzz/shared/widgets/ios_glass_navigation_button.dart';
import 'package:buzz/shared/widgets/playing_avatar_image.dart';
import 'package:buzz/shared/widgets/progressive_animated_avatar.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:image/image.dart' as image;
import 'package:image_picker/image_picker.dart';

import '../../helpers/widget_helpers.dart';

part 'profile_edit_page_test/motion_and_accessibility_tests.dart';
part 'profile_edit_page_test/image_selection_tests.dart';

const _editorControlBottomForTest = Grid.xl + Grid.xxs;

void main() {
  testWidgets('keeps crop Save disabled while dimensions decode', (
    tester,
  ) async {
    final bytes = Uint8List.fromList(
      image.encodePng(image.Image(width: 20, height: 10)),
    );
    await tester.pumpWidget(
      MaterialApp(
        home: ProfileAvatarCropPage(
          imageBytes: Future<Uint8List?>.value(bytes),
        ),
      ),
    );
    await tester.pump();
    await tester.pump();

    final saveButton = tester.widget<TextButton>(
      find.descendant(
        of: find.byKey(const ValueKey('avatar-crop-use-photo')),
        matching: find.byType(TextButton),
      ),
    );
    expect(saveButton.onPressed, isNull);
  });

  testWidgets('can open directly into the photo editor from Settings', (
    tester,
  ) async {
    await tester.pumpWidget(
      WidgetHelpers.testable(
        overrides: [profileProvider.overrideWith(_FakeProfileNotifier.new)],
        child: const ProfileEditPage(startInPhotoEditor: true),
      ),
    );
    await tester.pump();

    expect(find.text('Edit Photo'), findsOneWidget);
    expect(
      find.byKey(const ValueKey('profile-avatar-editor-page')),
      findsOneWidget,
    );
    expect(
      find.byKey(const ValueKey('profile-information-page')),
      findsNothing,
    );

    final entrance = find.byKey(
      const ValueKey('avatar-editor-entrance-transform'),
    );
    final initialOffset = tester
        .widget<Transform>(entrance)
        .transform
        .getTranslation()
        .y;
    final screenHeight =
        tester.view.physicalSize.height / tester.view.devicePixelRatio;
    final appBarHeight = frostedAppBarHeight(tester.element(entrance));
    final expectedOffset = appBarHeight + 96 - screenHeight / 2;
    expect(initialOffset, closeTo(expectedOffset, 0.01));

    await tester.pump(const Duration(milliseconds: 100));
    final midpointOffset = tester
        .widget<Transform>(entrance)
        .transform
        .getTranslation()
        .y;
    expect(midpointOffset, greaterThan(initialOffset));
    expect(midpointOffset, lessThan(0));

    await tester.pumpAndSettle();
    expect(
      tester.widget<Transform>(entrance).transform.getTranslation().y,
      closeTo(0, 0.01),
    );
    expect(
      tester
          .getCenter(find.byKey(const ValueKey('avatar-editor-fixed-preview')))
          .dy,
      closeTo(screenHeight / 2, 0.01),
    );
  });

  testWidgets('return motion eases out into the Settings avatar position', (
    tester,
  ) async {
    await tester.pumpWidget(
      ProviderScope(
        overrides: [profileProvider.overrideWith(_FakeProfileNotifier.new)],
        child: MaterialApp(
          theme: AppTheme.light(),
          home: Builder(
            builder: (context) => Scaffold(
              body: TextButton(
                onPressed: () => Navigator.of(context).push(
                  immediatePageRoute<void>(
                    builder: (_) =>
                        const ProfileEditPage(startInPhotoEditor: true),
                  ),
                ),
                child: const Text('Open editor'),
              ),
            ),
          ),
        ),
      ),
    );

    await tester.tap(find.text('Open editor'));
    await tester.pumpAndSettle();
    final entrance = find.byKey(
      const ValueKey('avatar-editor-entrance-transform'),
    );
    expect(
      tester.widget<Transform>(entrance).transform.getTranslation().y,
      closeTo(0, 0.01),
    );

    await tester.tap(find.byKey(const ValueKey('avatar-editor-back')));
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 50));
    final quarterOffset = tester
        .widget<Transform>(entrance)
        .transform
        .getTranslation()
        .y;
    final screenHeight =
        tester.view.physicalSize.height / tester.view.devicePixelRatio;
    final appBarHeight = frostedAppBarHeight(tester.element(entrance));
    final collapsedOffset = appBarHeight + 96 - screenHeight / 2;
    expect(quarterOffset, lessThan(collapsedOffset / 2));
    expect(quarterOffset, greaterThan(collapsedOffset));

    await tester.pumpAndSettle();
    expect(find.text('Open editor'), findsOneWidget);
    expect(
      find.byKey(const ValueKey('profile-avatar-editor-page')),
      findsNothing,
    );
  });

  testWidgets('uses the native segmented control for photo modes on iOS', (
    tester,
  ) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.iOS;

    await tester.pumpWidget(
      WidgetHelpers.testable(
        overrides: [profileProvider.overrideWith(_FakeProfileNotifier.new)],
        child: const ProfileEditPage(),
      ),
    );
    await tester.pumpAndSettle();
    await tester.tap(find.text('Edit Photo'));
    await tester.pump();

    expect(find.byType(IosNativeSegmentedControl), findsOneWidget);
    debugDefaultTargetPlatformOverride = null;
  });

  testWidgets('uses the native profile text form on iOS', (tester) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
    const channel = MethodChannel('buzz/profile_text_editor');
    MethodCall? receivedCall;
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(channel, (call) async {
          receivedCall = call;
          return 'Alice Native';
        });
    addTearDown(
      () => TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
          .setMockMethodCallHandler(channel, null),
    );
    final notifier = _FakeProfileNotifier();

    await tester.pumpWidget(
      ProviderScope(
        overrides: [profileProvider.overrideWith(() => notifier)],
        child: MaterialApp(
          theme: AppTheme.dark(),
          home: const Scaffold(body: ProfileEditPage()),
        ),
      ),
    );
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const ValueKey('profile-display-name-row')));
    await tester.pumpAndSettle();

    expect(receivedCall?.method, 'present');
    expect(receivedCall?.arguments, {
      'title': 'Display name',
      'initialValue': 'Alice',
      'placeholder': 'Display name',
      'multiline': false,
      'brightness': 'dark',
      'pageBackgroundArgb': AppTheme.dark().colorScheme.surfaceContainerHighest
          .toARGB32(),
      'containerBackgroundArgb': AppTheme.dark().colorScheme.surface.toARGB32(),
      'containerCornerRadius': Radii.container,
      'allowUnchangedSubmission': false,
    });
    expect(notifier.savedDisplayNames, ['Alice Native']);
    debugDefaultTargetPlatformOverride = null;
  });

  testWidgets('shows profile fields and saves each one from a sheet', (
    tester,
  ) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.android;
    addTearDown(() => debugDefaultTargetPlatformOverride = null);
    final notifier = _FakeProfileNotifier();
    await tester.pumpWidget(
      WidgetHelpers.testable(
        overrides: [profileProvider.overrideWith(() => notifier)],
        child: const ProfileEditPage(),
      ),
    );
    await tester.pumpAndSettle();

    expect(find.text('Edit Photo'), findsOneWidget);
    expect(find.text('Display name'), findsOneWidget);
    expect(find.text('Profile description'), findsOneWidget);
    expect(find.text('Alice'), findsOneWidget);
    expect(find.text('Building Buzz'), findsOneWidget);

    await tester.tap(find.byKey(const ValueKey('profile-display-name-row')));
    await tester.pumpAndSettle();
    await tester.enterText(
      find.byKey(const ValueKey('profile-field-input')),
      'Alice L',
    );
    await tester.pump();
    await tester.tap(find.byKey(const ValueKey('profile-field-save')));
    await tester.pumpAndSettle();

    expect(notifier.savedDisplayNames, ['Alice L']);
    expect(find.text('Alice L'), findsOneWidget);

    await tester.tap(find.byKey(const ValueKey('profile-description-row')));
    await tester.pumpAndSettle();
    final field = tester.widget<TextField>(
      find.byKey(const ValueKey('profile-field-input')),
    );
    expect(field.minLines, 4);
    await tester.enterText(
      find.byKey(const ValueKey('profile-field-input')),
      'Making collaboration feel effortless.',
    );
    await tester.pump();
    await tester.tap(find.byKey(const ValueKey('profile-field-save')));
    await tester.pumpAndSettle();

    expect(notifier.savedDescriptions, [
      'Making collaboration feel effortless.',
    ]);
    debugDefaultTargetPlatformOverride = null;
  });

  testWidgets('uses an icon-free grouped card with inset dividers', (
    tester,
  ) async {
    await tester.pumpWidget(
      WidgetHelpers.testable(
        overrides: [profileProvider.overrideWith(_FakeProfileNotifier.new)],
        child: const ProfileEditPage(),
      ),
    );
    await tester.pumpAndSettle();

    expect(
      find.descendant(
        of: find.byKey(const ValueKey('profile-info-card')),
        matching: find.byType(Icon),
      ),
      findsNWidgets(2),
    );
    expect(
      find.descendant(
        of: find.byKey(const ValueKey('profile-info-card')),
        matching: find.byType(Divider),
      ),
      findsOneWidget,
    );
  });

  testWidgets('uploads a selected photo and updates the profile', (
    tester,
  ) async {
    final notifier = _FakeProfileNotifier();
    final uploadService = _FakeMediaUploadService();
    addTearDown(uploadService.dispose);
    await tester.pumpWidget(
      WidgetHelpers.testable(
        overrides: [
          profileProvider.overrideWith(() => notifier),
          mediaUploadServiceProvider.overrideWithValue(uploadService),
        ],
        child: const ProfileEditPage(),
      ),
    );
    await tester.pumpAndSettle();

    await tester.tap(find.text('Edit Photo'));
    await tester.pumpAndSettle();
    expect(find.byKey(const ValueKey('avatar-mode-control')), findsOneWidget);
    expect(find.text('Image'), findsOneWidget);
    expect(find.text('Camera'), findsOneWidget);
    expect(find.text('Photo Library'), findsOneWidget);
    expect(find.text('Emoji'), findsOneWidget);
    expect(find.text('Animated'), findsOneWidget);
    expect(find.text('Photo'), findsNothing);
    final modeControl = find.byKey(const ValueKey('avatar-mode-control'));
    final modeIndicator = find.byKey(const ValueKey('avatar-mode-indicator'));
    final controlRect = tester.getRect(modeControl);
    final indicatorRect = tester.getRect(modeIndicator);
    expect(indicatorRect.left - controlRect.left, Grid.quarter);
    expect(indicatorRect.top - controlRect.top, Grid.quarter);
    expect(controlRect.bottom - indicatorRect.bottom, Grid.quarter);
    expect(
      controlRect.width - Grid.quarter * 2,
      closeTo(indicatorRect.width * 3, 0.01),
    );
    final indicatorDecoration =
        tester.widget<DecoratedBox>(modeIndicator).decoration as BoxDecoration;
    expect(indicatorDecoration.boxShadow, isNull);
    final preview = find.descendant(
      of: find.byKey(const ValueKey('avatar-editor-content')),
      matching: find.byType(AvatarImage),
    );
    expect(
      tester.getTopLeft(modeControl).dy,
      lessThan(tester.getTopLeft(preview).dy),
    );
    expect(tester.widget<AvatarImage>(preview).radius, 110);
    await tester.tap(find.text('Photo Library'));
    await _waitForAvatarCropToLoad(tester);
    expect(find.text('Position Photo'), findsOneWidget);
    final cancelButton = find.ancestor(
      of: find.text('Cancel'),
      matching: find.byType(TextButton),
    );
    final saveButton = find.byKey(const ValueKey('avatar-crop-use-photo'));
    expect(tester.getRect(cancelButton).left, Grid.gutter);
    expect(
      tester.getSize(find.text('Cancel')).width,
      lessThan(tester.getSize(cancelButton).width),
    );
    expect(
      tester.getSize(find.byType(Scaffold).last).width -
          tester.getRect(saveButton).right,
      Grid.gutter,
    );
    final cropViewerFinder = find.byKey(const ValueKey('avatar-crop-viewer'));
    final cropViewer = tester.widget<InteractiveViewer>(cropViewerFinder);
    final cropViewport = tester.getSize(cropViewerFinder);
    final cropDiameter = math.min(cropViewport.width, cropViewport.height);
    final expectedX = (cropViewport.width - cropDiameter) / 2;
    final expectedY = (cropViewport.height - cropDiameter) / 2;
    await tester.drag(cropViewerFinder, const Offset(1000, 1000));
    await tester.pump();
    expect(
      cropViewer.transformationController!.value.storage[12],
      closeTo(expectedX, 0.01),
    );
    expect(
      cropViewer.transformationController!.value.storage[13],
      closeTo(expectedY, 0.01),
    );
    await tester.tap(find.byKey(const ValueKey('avatar-crop-use-photo')));
    await _waitForAvatarCropToClose(tester);
    expect(notifier.savedAvatarUrls, isEmpty);
    expect(uploadService.uploadCount, 0);
    await tester.tap(find.byKey(const ValueKey('avatar-save')));
    await tester.pumpAndSettle();

    expect(notifier.savedAvatarUrls, ['https://relay.example/profile.png']);
    expect(uploadService.uploadCount, 1);
  });

  testWidgets('keeps a failed avatar draft visible and retries publish', (
    tester,
  ) async {
    final notifier = _FakeProfileNotifier(failedAvatarSaves: 1);
    final uploadService = _FakeMediaUploadService();
    addTearDown(uploadService.dispose);
    await tester.pumpWidget(
      WidgetHelpers.testable(
        overrides: [
          profileProvider.overrideWith(() => notifier),
          mediaUploadServiceProvider.overrideWithValue(uploadService),
        ],
        child: const ProfileEditPage(),
      ),
    );
    await tester.pumpAndSettle();
    await tester.tap(find.text('Edit Photo'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Photo Library'));
    await _waitForAvatarCropToLoad(tester);
    await tester.tap(find.byKey(const ValueKey('avatar-crop-use-photo')));
    await _waitForAvatarCropToClose(tester);

    await tester.tap(find.byKey(const ValueKey('avatar-save')));
    await tester.pumpAndSettle();
    expect(
      find.text("We couldn't save your profile photo. Try again."),
      findsOneWidget,
    );
    expect(
      find.byKey(const ValueKey('profile-avatar-editor-page')),
      findsOneWidget,
    );
    expect(uploadService.uploadCount, 1);

    await tester.tap(find.byKey(const ValueKey('avatar-save')));
    await tester.pumpAndSettle();
    expect(notifier.savedAvatarUrls, ['https://relay.example/profile.png']);
    expect(uploadService.uploadCount, 1);
  });

  testWidgets('centers every preview while controls fill the page gutters', (
    tester,
  ) async {
    tester.view.devicePixelRatio = 1;
    tester.view.physicalSize = const Size(800, 900);
    addTearDown(tester.view.resetDevicePixelRatio);
    addTearDown(tester.view.resetPhysicalSize);
    await tester.pumpWidget(
      WidgetHelpers.testable(
        overrides: [profileProvider.overrideWith(_FakeProfileNotifier.new)],
        child: const ProfileEditPage(),
      ),
    );
    await tester.pumpAndSettle();
    await tester.tap(find.text('Edit Photo'));
    await tester.pumpAndSettle();

    final screenSize = tester.view.physicalSize / tester.view.devicePixelRatio;
    final expectedContentWidth = screenSize.width - Grid.gutter * 2;
    expect(
      tester
          .getCenter(find.byKey(const ValueKey('avatar-editor-fixed-preview')))
          .dy,
      closeTo(screenSize.height / 2, 0.01),
    );
    expect(
      tester.getSize(find.byKey(const ValueKey(0))).width,
      closeTo(expectedContentWidth, 0.01),
    );
    expect(find.byKey(const ValueKey('image-source-camera')), findsOneWidget);
    expect(find.byKey(const ValueKey('image-source-library')), findsOneWidget);
    final cameraSurface = find.descendant(
      of: find.byKey(const ValueKey('image-source-camera')),
      matching: find.byType(AnimatedContainer),
    );
    final librarySurface = find.descendant(
      of: find.byKey(const ValueKey('image-source-library')),
      matching: find.byType(AnimatedContainer),
    );
    final imageControlGap =
        tester.getRect(librarySurface).left -
        tester.getRect(cameraSurface).right;
    expect(imageControlGap, greaterThan(Grid.twelve));
    expect(
      screenSize.height -
          tester
              .getRect(find.byKey(const ValueKey('image-source-camera')))
              .bottom,
      _editorControlBottomForTest,
    );

    await tester.tap(find.text('Emoji'));
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 150));
    expect(
      tester.getCenter(find.byKey(const ValueKey('emoji-avatar-preview'))).dy,
      closeTo(screenSize.height / 2 - 140, 0.01),
    );
    expect(
      tester
          .getSize(find.byKey(const ValueKey('emoji-avatar-picker-content')))
          .width,
      closeTo(expectedContentWidth, 0.01),
    );
    expect(
      find.byKey(const ValueKey('emoji-editor-background')),
      findsOneWidget,
    );
    expect(find.byKey(const ValueKey('emoji-editor-emoji')), findsOneWidget);
    expect(
      find.descendant(
        of: find.byKey(const ValueKey('emoji-avatar-picker-content')),
        matching: find.byType(AnimatedSwitcher),
      ),
      findsNothing,
    );
    final backgroundSurface = find.descendant(
      of: find.byKey(const ValueKey('emoji-editor-background')),
      matching: find.byType(AnimatedContainer),
    );
    final emojiSurface = find.descendant(
      of: find.byKey(const ValueKey('emoji-editor-emoji')),
      matching: find.byType(AnimatedContainer),
    );
    expect(
      tester.getRect(emojiSurface).left -
          tester.getRect(backgroundSurface).right,
      closeTo(imageControlGap, 0.01),
    );
    expect(
      screenSize.height -
          tester
              .getRect(find.byKey(const ValueKey('emoji-editor-background')))
              .bottom,
      _editorControlBottomForTest,
    );

    final expandedPickerHeight = tester
        .getSize(find.byKey(const ValueKey('emoji-avatar-picker-content')))
        .height;
    await tester.tap(find.byKey(const ValueKey('emoji-editor-background')));
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 150));
    expect(
      tester.getCenter(find.byKey(const ValueKey('emoji-avatar-preview'))).dy,
      closeTo(screenSize.height / 2 - 140, 0.01),
    );
    expect(
      tester
          .getSize(find.byKey(const ValueKey('emoji-avatar-picker-content')))
          .height,
      expandedPickerHeight,
    );
    expect(
      tester.getCenter(find.byKey(const ValueKey('emoji-background-editor'))),
      tester.getCenter(
        find.byKey(const ValueKey('emoji-background-editor-alignment')),
      ),
    );

    await tester.tap(find.text('Animated'));
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 150));
    expect(
      tester
          .getCenter(
            find.byKey(const ValueKey('animated-avatar-capture-preview')),
          )
          .dy,
      closeTo(screenSize.height / 2, 0.01),
    );
    expect(
      tester.getSize(find.byKey(const ValueKey(2))).width,
      closeTo(expectedContentWidth, 0.01),
    );
    final recordButton = find.byKey(
      const ValueKey('animated-avatar-record-morph'),
    );
    expect(tester.getSize(recordButton).height, 64);
    expect(
      tester.getSize(recordButton).width,
      closeTo(expectedContentWidth, 0.01),
    );
    final recordMaterial = tester.widget<Material>(
      find.descendant(of: recordButton, matching: find.byType(Material)).first,
    );
    expect(
      recordMaterial.borderRadius,
      const BorderRadius.all(Radius.circular(Radii.full)),
    );
    expect(
      screenSize.height - tester.getRect(recordButton).bottom,
      _editorControlBottomForTest,
    );
    expect(
      find.byKey(const ValueKey('avatar-mode-transition-transform')),
      findsNothing,
    );
  });

  testWidgets('waits for the photo picker before opening Position Photo', (
    tester,
  ) async {
    final uploadService = _FakeMediaUploadService(delayGallery: true);
    addTearDown(uploadService.dispose);
    await tester.pumpWidget(
      WidgetHelpers.testable(
        overrides: [
          profileProvider.overrideWith(_FakeProfileNotifier.new),
          mediaUploadServiceProvider.overrideWithValue(uploadService),
        ],
        child: const ProfileEditPage(),
      ),
    );
    await tester.pumpAndSettle();
    await tester.tap(find.text('Edit Photo'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Photo Library'));
    await tester.pump();

    expect(find.text('Position Photo'), findsNothing);
    expect(
      find.byKey(const ValueKey('avatar-image-loading-overlay')),
      findsOneWidget,
    );

    uploadService.completeGallerySelection();
    await _waitForAvatarCropToLoad(tester);
    expect(find.text('Position Photo'), findsOneWidget);
  });

  testWidgets('captures a camera photo and updates the profile', (
    tester,
  ) async {
    final notifier = _FakeProfileNotifier();
    final uploadService = _FakeMediaUploadService();
    addTearDown(uploadService.dispose);
    await tester.pumpWidget(
      WidgetHelpers.testable(
        overrides: [
          profileProvider.overrideWith(() => notifier),
          mediaUploadServiceProvider.overrideWithValue(uploadService),
        ],
        child: ProfileEditPage(
          imageAvatarCaptureBuilder:
              ({required height, required onAccepted, required onClosed}) =>
                  _FakeImageAvatarCapture(
                    onAccepted: onAccepted,
                    onClosed: onClosed,
                  ),
        ),
      ),
    );
    await tester.pumpAndSettle();

    await tester.tap(find.text('Edit Photo'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Camera'));
    await tester.pump();
    expect(find.byKey(const ValueKey('fake-image-camera')), findsOneWidget);
    await tester.tap(find.byKey(const ValueKey('fake-image-camera-accept')));
    await tester.pumpAndSettle();
    expect(notifier.savedAvatarUrls, isEmpty);
    await tester.tap(find.byKey(const ValueKey('avatar-save')));
    await tester.pumpAndSettle();

    expect(notifier.savedAvatarUrls, ['https://relay.example/profile.png']);
  });

  testWidgets('keeps the local animation visible after profile Save', (
    tester,
  ) async {
    final notifier = _FakeProfileNotifier(updatesProfileState: true);
    final uploadService = _FakeMediaUploadService();
    addTearDown(uploadService.dispose);
    final frame = Uint8List.fromList(
      image.encodePng(
        image.Image(width: 8, height: 8)..setPixelRgba(4, 4, 255, 0, 0, 255),
      ),
    );
    await tester.pumpWidget(
      WidgetHelpers.testable(
        overrides: [
          profileProvider.overrideWith(() => notifier),
          mediaUploadServiceProvider.overrideWithValue(uploadService),
        ],
        child: ProfileEditPage(
          animatedAvatarCaptureBuilder:
              ({required height, required onPrepareChanged}) {
                WidgetsBinding.instance.addPostFrameCallback((_) {
                  onPrepareChanged(
                    () async => ProfileAnimatedAvatarDraft(
                      animation: frame,
                      poster: frame,
                    ),
                  );
                });
                return const SizedBox(
                  key: ValueKey('fake-animated-avatar-review'),
                );
              },
        ),
      ),
    );
    await tester.pumpAndSettle();

    await tester.tap(find.text('Edit Photo'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Animated'));
    await tester.pump();
    await tester.pump();
    await tester.tap(find.byKey(const ValueKey('avatar-save')));
    await tester.pumpAndSettle();

    expect(
      find.byKey(const ValueKey('progressive-animated-avatar-local-handoff')),
      findsOneWidget,
    );
    expect(notifier.savedAvatarUrls.single, contains('#buzz-anim='));
  });

  testWidgets('saves a desktop-compatible emoji avatar', (tester) async {
    final notifier = _FakeProfileNotifier();
    await tester.pumpWidget(
      WidgetHelpers.testable(
        overrides: [profileProvider.overrideWith(() => notifier)],
        child: const ProfileEditPage(),
      ),
    );
    await tester.pumpAndSettle();

    await tester.tap(find.text('Edit Photo'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Emoji'));
    await tester.pump();
    expect(find.byKey(const ValueKey('emoji-avatar-search')), findsOneWidget);
    expect(
      find.byKey(const ValueKey('emoji-avatar-skin-tone')),
      findsOneWidget,
    );
    expect(
      tester.getSize(find.byKey(const ValueKey('emoji-avatar-preview'))),
      const Size.square(220),
    );
    await tester.enterText(
      find.byKey(const ValueKey('emoji-avatar-search')),
      'rocket',
    );
    await tester.pump(const Duration(milliseconds: 300));
    final focusedSearch = tester.widget<TextField>(
      find.byKey(const ValueKey('emoji-avatar-search')),
    );
    final focusedBorder =
        focusedSearch.decoration?.focusedBorder! as OutlineInputBorder;
    expect(
      focusedBorder.borderRadius,
      const BorderRadius.all(Radius.circular(Radii.full)),
    );
    expect(
      tester
          .widget<TextField>(find.byKey(const ValueKey('emoji-avatar-search')))
          .controller
          ?.text,
      'rocket',
    );
    await tester.tap(find.byKey(const ValueKey('emoji-avatar-skin-tone')));
    await tester.pump(const Duration(milliseconds: 300));
    expect(find.text('Dark'), findsOneWidget);
    await tester.tapAt(const Offset(8, 8));
    await tester.pump(const Duration(milliseconds: 300));
    await tester.enterText(
      find.byKey(const ValueKey('emoji-avatar-search')),
      '',
    );
    await tester.pump(const Duration(milliseconds: 300));
    final previewFinder = find.byKey(const ValueKey('emoji-avatar-preview'));
    final previewBefore = tester.widget<AnimatedContainer>(previewFinder);
    final decorationBefore = previewBefore.decoration! as BoxDecoration;
    expect(decorationBefore.color, isNot(Colors.white));
    final glyph = tester.widget<NativeEmojiGlyph>(
      find.descendant(
        of: previewFinder,
        matching: find.byType(NativeEmojiGlyph),
      ),
    );
    expect(glyph.size, closeTo(220 * 258 / 512, 0.01));
    final nextColorIndex = emojiAvatarColors.indexWhere(
      (value) => Color(value) != decorationBefore.color,
    );
    final nextColor = find.byKey(
      ValueKey('emoji-avatar-color-$nextColorIndex'),
    );
    await tester.tap(find.byKey(const ValueKey('emoji-editor-background')));
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 200));
    await tester.tap(nextColor);
    await tester.pump();
    final previewAfter = tester.widget<AnimatedContainer>(previewFinder);
    expect(previewAfter.decoration, isNot(previewBefore.decoration));
    await tester.tap(find.byKey(const ValueKey('avatar-save')));
    await tester.pumpAndSettle();

    final avatarUrl = notifier.savedAvatarUrls.single;
    expect(avatarUrl, startsWith('data:image/svg+xml,'));
    expect(Uri.decodeComponent(avatarUrl), contains('😊'));
    expect(
      find.byKey(const ValueKey('profile-information-page')),
      findsOneWidget,
    );
    expect(
      find.byKey(const ValueKey('profile-avatar-editor-page')),
      findsNothing,
    );
  });

  testWidgets('saves the displayed default emoji without another selection', (
    tester,
  ) async {
    final notifier = _FakeProfileNotifier();
    await tester.pumpWidget(
      WidgetHelpers.testable(
        overrides: [profileProvider.overrideWith(() => notifier)],
        child: const ProfileEditPage(),
      ),
    );
    await tester.pumpAndSettle();

    await tester.tap(find.text('Edit Photo'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Emoji'));
    await tester.pump(const Duration(milliseconds: 250));
    await tester.tap(find.byKey(const ValueKey('avatar-save')));
    await tester.pumpAndSettle();

    expect(notifier.savedAvatarUrls, hasLength(1));
    expect(notifier.savedAvatarUrls.single, startsWith('data:image/svg+xml,'));
    expect(
      Uri.decodeComponent(notifier.savedAvatarUrls.single),
      contains('😊'),
    );
  });

  testWidgets('keeps emoji drafts scoped to the emoji mode', (tester) async {
    final notifier = _FakeProfileNotifier();
    await tester.pumpWidget(
      WidgetHelpers.testable(
        overrides: [profileProvider.overrideWith(() => notifier)],
        child: const ProfileEditPage(),
      ),
    );
    await tester.pumpAndSettle();
    await tester.tap(find.text('Edit Photo'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Emoji'));
    await tester.pump(const Duration(milliseconds: 500));

    await tester.tap(find.byKey(const ValueKey('emoji-editor-background')));
    await tester.pump(const Duration(milliseconds: 200));
    await tester.tap(find.byKey(const ValueKey('emoji-avatar-color-1')));
    await tester.pump();
    await tester.tap(find.text('Image'));
    await tester.pump(const Duration(milliseconds: 500));
    await tester.tap(find.byKey(const ValueKey('avatar-save')));
    await tester.pump();

    expect(notifier.savedAvatarUrls, isEmpty);
    expect(
      find.byKey(const ValueKey('profile-avatar-editor-page')),
      findsOneWidget,
    );
  });

  runProfileEditMotionAndAccessibilityTests();
  runProfileEditImageSelectionTests();
}

Future<void> _waitForAvatarCropToClose(WidgetTester tester) async {
  final cropPage = find.byKey(const ValueKey('avatar-crop-viewer'));
  for (var attempt = 0; attempt < 100; attempt += 1) {
    await tester.runAsync(
      () => Future<void>.delayed(const Duration(milliseconds: 25)),
    );
    await tester.pump(const Duration(milliseconds: 25));
    if (cropPage.evaluate().isEmpty) {
      await tester.pump(const Duration(milliseconds: 250));
      return;
    }
  }
  fail('Avatar crop did not complete within 5 seconds.');
}

Future<void> _waitForAvatarCropToLoad(WidgetTester tester) async {
  final cropViewer = find.byKey(const ValueKey('avatar-crop-viewer'));
  for (var attempt = 0; attempt < 100; attempt += 1) {
    await tester.runAsync(
      () => Future<void>.delayed(const Duration(milliseconds: 25)),
    );
    await tester.pump(const Duration(milliseconds: 25));
    if (cropViewer.evaluate().isNotEmpty) {
      await tester.pumpAndSettle();
      return;
    }
  }
  fail('Avatar crop did not load within 5 seconds.');
}

class _FakeProfileNotifier extends ProfileNotifier {
  _FakeProfileNotifier({
    this.profile = const UserProfile(
      pubkey: 'aabb',
      displayName: 'Alice',
      about: 'Building Buzz',
    ),
    this.failedAvatarSaves = 0,
    this.updatesProfileState = false,
  });

  final UserProfile profile;
  int failedAvatarSaves;
  final bool updatesProfileState;
  final savedDisplayNames = <String>[];
  final savedDescriptions = <String>[];
  final savedAvatarUrls = <String>[];

  @override
  Future<UserProfile?> build() async => profile;

  @override
  Future<void> updateDisplayName(String displayName) async {
    savedDisplayNames.add(displayName);
    final current = state.requireValue!;
    state = AsyncData(
      UserProfile(
        pubkey: current.pubkey,
        displayName: displayName,
        avatarUrl: current.avatarUrl,
        about: current.about,
        nip05Handle: current.nip05Handle,
      ),
    );
  }

  @override
  Future<void> updateAbout(String about) async {
    savedDescriptions.add(about);
    final current = state.requireValue!;
    state = AsyncData(
      UserProfile(
        pubkey: current.pubkey,
        displayName: current.displayName,
        avatarUrl: current.avatarUrl,
        about: about,
        nip05Handle: current.nip05Handle,
      ),
    );
  }

  @override
  Future<void> updateAvatarUrl(String avatarUrl) async {
    if (failedAvatarSaves > 0) {
      failedAvatarSaves--;
      throw Exception('profile publish failed');
    }
    savedAvatarUrls.add(avatarUrl);
    if (updatesProfileState) {
      final current = state.requireValue!;
      state = AsyncData(
        UserProfile(
          pubkey: current.pubkey,
          displayName: current.displayName,
          avatarUrl: avatarUrl,
          about: current.about,
          nip05Handle: current.nip05Handle,
        ),
      );
    }
  }
}

class _FakeMediaUploadService extends MediaUploadService {
  _FakeMediaUploadService({
    this.delayGallery = false,
    this.failImagePreparation = false,
  }) : super(
         baseUrl: 'https://relay.example',
         nsec: null,
         pickGalleryImage: () async => null,
         pickGalleryVideo: () async => null,
       );

  var _camera = false;
  final bool delayGallery;
  final bool failImagePreparation;
  final _gallerySelection = Completer<XFile?>();
  int uploadCount = 0;

  XFile _image() => XFile.fromData(
    image.encodePng(image.Image(width: 8, height: 8)),
    mimeType: 'image/png',
    name: 'avatar.png',
  );

  @override
  Future<XFile?> pickGalleryImage() async {
    _camera = false;
    if (delayGallery) return _gallerySelection.future;
    return _image();
  }

  void completeGallerySelection() => _gallerySelection.complete(_image());

  @override
  Future<XFile?> captureImage() async {
    _camera = true;
    return _image();
  }

  @override
  Future<Uint8List> prepareImageBytes(XFile image) async {
    if (failImagePreparation) throw Exception('image preparation failed');
    return image.readAsBytes();
  }

  @override
  Future<BlobDescriptor> uploadBytes(
    Uint8List bytes, {
    required String mimeType,
    ValueChanged<double>? onProgress,
    UploadCancellationToken? cancellationToken,
  }) async {
    uploadCount++;
    return BlobDescriptor(
      url: _camera
          ? 'https://relay.example/camera.png'
          : 'https://relay.example/profile.png',
      sha256: _camera ? 'camera-hash' : 'hash',
      size: bytes.length,
      type: mimeType,
      uploaded: 1,
    );
  }
}

class _FakeImageAvatarCapture extends StatelessWidget {
  const _FakeImageAvatarCapture({
    required this.onAccepted,
    required this.onClosed,
  });

  final ValueChanged<Uint8List> onAccepted;
  final VoidCallback onClosed;

  @override
  Widget build(BuildContext context) => Row(
    key: const ValueKey('fake-image-camera'),
    children: [
      TextButton(
        key: const ValueKey('fake-image-camera-close'),
        onPressed: onClosed,
        child: const Text('Close fake camera'),
      ),
      TextButton(
        key: const ValueKey('fake-image-camera-accept'),
        onPressed: () => onAccepted(
          Uint8List.fromList(image.encodeJpg(image.Image(width: 8, height: 8))),
        ),
        child: const Text('Accept fake photo'),
      ),
    ],
  );
}
