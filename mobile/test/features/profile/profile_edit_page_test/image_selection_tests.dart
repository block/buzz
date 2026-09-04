part of '../profile_edit_page_test.dart';

double _paintedWidth(WidgetTester tester, String key) {
  final box = tester.renderObject<RenderBox>(find.byKey(ValueKey(key)));
  final left = box.localToGlobal(Offset.zero);
  final right = box.localToGlobal(Offset(box.size.width, 0));
  return (right - left).distance;
}

void runProfileEditImageSelectionTests() {
  testWidgets('uses glass image source controls on iOS', (tester) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
    addTearDown(() => debugDefaultTargetPlatformOverride = null);
    await tester.pumpWidget(
      WidgetHelpers.testable(
        overrides: [profileProvider.overrideWith(_FakeProfileNotifier.new)],
        child: const ProfileEditPage(startInPhotoEditor: true),
      ),
    );
    await tester.pumpAndSettle();

    final nativeControls = tester
        .widgetList<UiKitView>(find.byType(UiKitView))
        .where((view) => view.viewType == IosGlassNavigationButton.viewType)
        .map((view) => view.creationParams as Map<String, Object>)
        .toList();
    expect(nativeControls.any((params) => params['icon'] == 'camera'), isTrue);
    expect(
      nativeControls.any((params) => params['icon'] == 'photoLibrary'),
      isTrue,
    );
    debugDefaultTargetPlatformOverride = null;
  });

  testWidgets('opens the inline camera around the existing avatar center', (
    tester,
  ) async {
    await tester.pumpWidget(
      WidgetHelpers.testable(
        overrides: [profileProvider.overrideWith(_FakeProfileNotifier.new)],
        child: const ProfileEditPage(startInPhotoEditor: true),
      ),
    );
    await tester.pumpAndSettle();

    final avatarCenter = tester.getCenter(
      find.byKey(const ValueKey('avatar-editor-fixed-preview')),
    );
    await tester.tap(find.byKey(const ValueKey('image-source-camera')));
    await tester.pump();

    expect(
      tester.getCenter(find.byKey(const ValueKey('image-camera-preview-size'))),
      avatarCenter,
    );
  });

  testWidgets('expands the avatar with the controls while camera loads', (
    tester,
  ) async {
    await tester.pumpWidget(
      WidgetHelpers.testable(
        child: Scaffold(
          body: SizedBox(
            height: 400,
            child: ImageAvatarCapture(
              height: 400,
              onAccepted: (_) {},
              initialPreview: const SizedBox.square(
                dimension: 220,
                child: ColoredBox(
                  key: ValueKey('existing-avatar-preview'),
                  color: Colors.pink,
                ),
              ),
              onClosed: () {},
              loadCameras: () async => const [],
            ),
          ),
        ),
      ),
    );

    final preview = find.byKey(const ValueKey('image-camera-preview-size'));
    expect(tester.getSize(preview), const Size.square(220));
    expect(tester.getCenter(preview).dy, imageAvatarCameraPreviewSize / 2);
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 90));
    final midSize = tester.getSize(preview).width;
    expect(midSize, greaterThan(220));
    expect(midSize, lessThan(275));
    expect(
      _paintedWidth(tester, 'existing-avatar-preview'),
      closeTo(midSize, 1),
    );
    expect(tester.getCenter(preview).dy, imageAvatarCameraPreviewSize / 2);
    await tester.pump(const Duration(milliseconds: 90));
    expect(tester.getSize(preview), const Size.square(275));
    expect(_paintedWidth(tester, 'existing-avatar-preview'), closeTo(275, 1));
    expect(
      find.byKey(const ValueKey('existing-avatar-preview')),
      findsOneWidget,
    );
    expect(
      tester.getSize(find.byKey(const ValueKey('image-camera-left-action'))),
      const Size.square(64),
    );
    expect(
      tester.getSize(find.byKey(const ValueKey('image-camera-right-action'))),
      const Size.square(64),
    );
    expect(
      tester.getSize(find.byKey(const ValueKey('image-camera-shutter-morph'))),
      const Size.square(100),
    );
    expect(find.bySemanticsLabel('Close camera'), findsOneWidget);
    expect(find.bySemanticsLabel('Flip camera'), findsOneWidget);
    expect(find.bySemanticsLabel('Take photo'), findsOneWidget);
  });

  testWidgets('reverses the camera controls before closing', (tester) async {
    var closed = false;
    await tester.pumpWidget(
      WidgetHelpers.testable(
        child: Scaffold(
          body: SizedBox(
            height: 400,
            child: ImageAvatarCapture(
              height: 400,
              onAccepted: (_) {},
              onClosed: () => closed = true,
              loadCameras: () async => const [],
            ),
          ),
        ),
      ),
    );
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 180));
    final leftAction = find.byKey(const ValueKey('image-camera-left-action'));
    final rightAction = find.byKey(const ValueKey('image-camera-right-action'));
    final expandedDistance =
        tester.getCenter(rightAction).dx - tester.getCenter(leftAction).dx;

    tester
        .widget<InkWell>(
          find.descendant(of: leftAction, matching: find.byType(InkWell)),
        )
        .onTap!();
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 90));
    final midDistance =
        tester.getCenter(rightAction).dx - tester.getCenter(leftAction).dx;
    expect(midDistance, lessThan(expandedDistance));
    expect(
      find.byKey(const ValueKey('camera-action-icon-camera')),
      findsOneWidget,
    );
    expect(
      find.byKey(const ValueKey('camera-action-icon-photoLibrary')),
      findsOneWidget,
    );
    expect(find.text('Camera'), findsOneWidget);
    expect(find.text('Photo Library'), findsOneWidget);
    final cameraIcon = tester.widget<Icon>(
      find.byKey(const ValueKey('camera-action-icon-camera')),
    );
    expect(cameraIcon.color?.a, 1);
    expect(closed, isFalse);

    await tester.pump(const Duration(milliseconds: 60));
    expect(
      tester
          .widget<Opacity>(
            find.byKey(const ValueKey('image-camera-shutter-exit-opacity')),
          )
          .opacity,
      0,
    );
    expect(closed, isFalse);

    await tester.pump(const Duration(milliseconds: 30));
    expect(closed, isTrue);
    expect(
      tester.getSize(find.byKey(const ValueKey('image-camera-preview-size'))),
      const Size.square(220),
    );
  });

  test('front camera output keeps the mirrored preview orientation', () {
    final source = image.Image(width: 2, height: 2);
    for (var y = 0; y < 2; y++) {
      source.setPixelRgb(0, y, 255, 0, 0);
      source.setPixelRgb(1, y, 0, 0, 255);
    }
    final encoded = Uint8List.fromList(image.encodePng(source));

    final regular = image.decodeJpg(
      prepareCameraImageForTesting(encoded, mirror: false),
    )!;
    final mirrored = image.decodeJpg(
      prepareCameraImageForTesting(encoded, mirror: true),
    )!;
    final regularLeft = regular.getPixel(40, 256);
    final mirroredLeft = mirrored.getPixel(40, 256);

    expect(regularLeft.r, greaterThan(regularLeft.b));
    expect(mirroredLeft.b, greaterThan(mirroredLeft.r));
  });

  testWidgets('provides haptics when closing or accepting a camera photo', (
    tester,
  ) async {
    final haptics = <Object?>[];
    tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
      SystemChannels.platform,
      (call) async {
        if (call.method == 'HapticFeedback.vibrate') {
          haptics.add(call.arguments);
        }
        return null;
      },
    );
    addTearDown(
      () => tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
        SystemChannels.platform,
        null,
      ),
    );

    await tester.pumpWidget(
      WidgetHelpers.testable(
        child: Scaffold(
          body: ImageAvatarCapture(
            key: const ValueKey('close-haptic-camera'),
            height: 400,
            onAccepted: (_) {},
            onClosed: () {},
            loadCameras: () async => const [],
          ),
        ),
      ),
    );
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 180));
    final closeAction = find.byKey(const ValueKey('image-camera-left-action'));
    tester
        .widget<InkWell>(
          find.descendant(of: closeAction, matching: find.byType(InkWell)),
        )
        .onTap!();
    await tester.pump();
    expect(haptics, contains('HapticFeedbackType.selectionClick'));
    await tester.pump(const Duration(milliseconds: 180));

    final bytes = Uint8List.fromList(
      image.encodeJpg(image.Image(width: 8, height: 8)),
    );
    await tester.pumpWidget(
      WidgetHelpers.testable(
        child: Scaffold(
          body: ImageAvatarCapture(
            key: const ValueKey('accept-haptic-camera'),
            height: 400,
            initialCapturedBytes: bytes,
            onAccepted: (_) {},
            onClosed: () {},
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();
    await tester.tap(find.text('Use Photo'));
    await tester.pump();
    expect(haptics, contains('HapticFeedbackType.mediumImpact'));
    await tester.pump(const Duration(milliseconds: 180));
  });

  testWidgets('reviews a captured photo before scaling down to accept it', (
    tester,
  ) async {
    final bytes = Uint8List.fromList(
      image.encodeJpg(image.Image(width: 8, height: 8)),
    );
    Uint8List? accepted;
    await tester.pumpWidget(
      WidgetHelpers.testable(
        child: Scaffold(
          body: SizedBox(
            height: 400,
            child: ImageAvatarCapture(
              height: 400,
              initialCapturedBytes: bytes,
              onAccepted: (value) => accepted = value,
              onClosed: () {},
            ),
          ),
        ),
      ),
    );
    final preview = find.byKey(const ValueKey('image-camera-preview-size'));
    final compactCenter = tester.getCenter(preview);
    expect(tester.getSize(preview), const Size.square(220));
    await tester.pumpAndSettle();

    expect(tester.getSize(preview), const Size.square(275));
    expect(tester.getCenter(preview), compactCenter);
    expect(find.text('Retry'), findsOneWidget);
    expect(find.text('Use Photo'), findsOneWidget);
    expect(
      tester.getSize(find.byKey(const ValueKey('image-camera-left-action'))),
      const Size(112, 64),
    );
    expect(
      tester.getSize(find.byKey(const ValueKey('image-camera-right-action'))),
      const Size(112, 64),
    );
    final leftRect = tester.getRect(
      find.byKey(const ValueKey('image-camera-left-action')),
    );
    final rightRect = tester.getRect(
      find.byKey(const ValueKey('image-camera-right-action')),
    );
    expect(rightRect.left - leftRect.right, 12);

    await tester.tap(find.text('Use Photo'));
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 90));
    final midSize = tester
        .getSize(find.byKey(const ValueKey('image-camera-preview-size')))
        .width;
    expect(midSize, greaterThan(220));
    expect(midSize, lessThan(275));
    expect(accepted, isNull);
    await tester.pump(const Duration(milliseconds: 90));
    expect(accepted, same(bytes));
    expect(
      tester.getSize(find.byKey(const ValueKey('image-camera-preview-size'))),
      const Size.square(220),
    );
  });

  testWidgets('retry moves the review actions apart again', (tester) async {
    final bytes = Uint8List.fromList(
      image.encodeJpg(image.Image(width: 8, height: 8)),
    );
    await tester.pumpWidget(
      WidgetHelpers.testable(
        child: Scaffold(
          body: SizedBox(
            height: 400,
            child: ImageAvatarCapture(
              height: 400,
              initialCapturedBytes: bytes,
              onAccepted: (_) {},
              onClosed: () {},
              loadCameras: () async => const [],
            ),
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();
    final leftAction = find.byKey(const ValueKey('image-camera-left-action'));
    final rightAction = find.byKey(const ValueKey('image-camera-right-action'));
    final reviewGap =
        tester.getRect(rightAction).left - tester.getRect(leftAction).right;

    await tester.tap(find.text('Retry'));
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 180));

    final cameraGap =
        tester.getRect(rightAction).left - tester.getRect(leftAction).right;
    expect(reviewGap, 12);
    expect(cameraGap, greaterThan(reviewGap));
    expect(find.bySemanticsLabel('Close camera'), findsOneWidget);
    expect(find.bySemanticsLabel('Flip camera'), findsOneWidget);
  });

  testWidgets('clears gallery errors when opening the inline camera', (
    tester,
  ) async {
    final uploadService = _FakeMediaUploadService(failImagePreparation: true);
    addTearDown(uploadService.dispose);
    await tester.pumpWidget(
      WidgetHelpers.testable(
        overrides: [
          profileProvider.overrideWith(_FakeProfileNotifier.new),
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
    await tester.tap(find.byKey(const ValueKey('image-source-library')));
    await tester.pumpAndSettle();

    expect(
      find.text("We couldn't prepare that photo. Try again."),
      findsOneWidget,
    );
    await tester.tap(find.byKey(const ValueKey('image-source-camera')));
    await tester.pump();

    expect(find.byKey(const ValueKey('fake-image-camera')), findsOneWidget);
    expect(
      find.text("We couldn't prepare that photo. Try again."),
      findsNothing,
    );
    await tester.tap(find.byKey(const ValueKey('fake-image-camera-accept')));
    await tester.pumpAndSettle();

    expect(
      find.text("We couldn't prepare that photo. Try again."),
      findsNothing,
    );
  });

  testWidgets('accepts an inline camera photo before enabling profile Save', (
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
    await tester.tap(find.byKey(const ValueKey('image-source-camera')));
    await tester.pump();

    expect(find.byKey(const ValueKey('fake-image-camera')), findsOneWidget);
    expect(find.byKey(const ValueKey('image-source-library')), findsNothing);
    await tester.tap(find.byKey(const ValueKey('fake-image-camera-accept')));
    await tester.pumpAndSettle();

    expect(find.byKey(const ValueKey('fake-image-camera')), findsNothing);
    expect(find.byKey(const ValueKey('image-source-camera')), findsOneWidget);
    expect(find.byKey(const ValueKey('image-source-library')), findsOneWidget);
    await tester.tap(find.byKey(const ValueKey('avatar-save')));
    await tester.pumpAndSettle();

    expect(notifier.savedAvatarUrls, ['https://relay.example/profile.png']);
    expect(uploadService.uploadCount, 1);
  });

  testWidgets('duplicate avatar Back taps pop only the editor route', (
    tester,
  ) async {
    await tester.pumpWidget(
      WidgetHelpers.testable(
        overrides: [profileProvider.overrideWith(_FakeProfileNotifier.new)],
        child: Builder(
          builder: (context) => Scaffold(
            body: TextButton(
              onPressed: () => unawaited(
                Navigator.of(context).push<void>(
                  MaterialPageRoute<void>(
                    builder: (_) =>
                        const ProfileEditPage(startInPhotoEditor: true),
                  ),
                ),
              ),
              child: const Text('Open profile photo'),
            ),
          ),
        ),
      ),
    );
    await tester.tap(find.text('Open profile photo'));
    await tester.pumpAndSettle();

    final back = find.byKey(const ValueKey('avatar-editor-back'));
    await tester.tap(back);
    await tester.tap(back);
    await tester.pumpAndSettle();

    expect(find.text('Open profile photo'), findsOneWidget);
  });

  testWidgets('photo crop exposes accessible move and zoom actions', (
    tester,
  ) async {
    final semantics = tester.ensureSemantics();
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
    await _waitForAvatarCropToLoad(tester);

    final cropSemantics = tester.widget<Semantics>(
      find.byWidgetPredicate(
        (widget) =>
            widget is Semantics && widget.properties.label == 'Photo crop',
      ),
    );
    final actions = cropSemantics.properties.customSemanticsActions!;
    expect(
      actions.keys.map((action) => action.label),
      containsAll([
        'Move left',
        'Move right',
        'Move up',
        'Move down',
        'Zoom in',
      ]),
    );
    final viewer = tester.widget<InteractiveViewer>(
      find.byKey(const ValueKey('avatar-crop-viewer')),
    );
    actions.entries.firstWhere((entry) => entry.key.label == 'Zoom in').value();
    await tester.pump();
    expect(viewer.transformationController!.value.getMaxScaleOnAxis(), 1.1);
    semantics.dispose();
  });

  testWidgets('seeds emoji editing from the current avatar', (tester) async {
    final avatarUrl = emojiAvatarDataUrl('🦝', emojiAvatarColors[11]);
    final notifier = _FakeProfileNotifier(
      profile: UserProfile(
        pubkey: 'aabb',
        displayName: 'Alice',
        avatarUrl: avatarUrl,
      ),
    );
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

    final preview = find.byKey(const ValueKey('emoji-avatar-preview'));
    expect(
      tester
          .widget<NativeEmojiGlyph>(
            find.descendant(
              of: preview,
              matching: find.byType(NativeEmojiGlyph),
            ),
          )
          .emoji,
      '🦝',
    );
    expect(
      (tester.widget<AnimatedContainer>(preview).decoration! as BoxDecoration)
          .color,
      Color(emojiAvatarColors[11]),
    );
    await tester.tap(find.byKey(const ValueKey('avatar-save')));
    await tester.pumpAndSettle();
    expect(notifier.savedAvatarUrls, [avatarUrl]);
  });

  testWidgets('seeds the skin-tone filter from the current emoji', (
    tester,
  ) async {
    const variants = [
      EmojiEntry(
        id: '+1',
        name: 'Thumbs Up',
        keywords: ['thumb'],
        native: '👍',
        categoryId: 'people',
      ),
      EmojiEntry(
        id: '+1',
        name: 'Thumbs Up',
        keywords: ['thumb'],
        native: '👍🏻',
        categoryId: 'people',
        skinIndex: 1,
      ),
      EmojiEntry(
        id: '+1',
        name: 'Thumbs Up',
        keywords: ['thumb'],
        native: '👍🏼',
        categoryId: 'people',
        skinIndex: 2,
      ),
      EmojiEntry(
        id: '+1',
        name: 'Thumbs Up',
        keywords: ['thumb'],
        native: '👍🏽',
        categoryId: 'people',
        skinIndex: 3,
      ),
      EmojiEntry(
        id: '+1',
        name: 'Thumbs Up',
        keywords: ['thumb'],
        native: '👍🏾',
        categoryId: 'people',
        skinIndex: 4,
      ),
      EmojiEntry(
        id: '+1',
        name: 'Thumbs Up',
        keywords: ['thumb'],
        native: '👍🏿',
        categoryId: 'people',
        skinIndex: 5,
      ),
    ];
    const dataset = EmojiDataset(
      categories: [EmojiCategory(id: 'people', emoji: variants)],
      all: variants,
      nativeToShortcode: {'👍🏽': ':+1:'},
    );
    final avatarUrl = emojiAvatarDataUrl('👍🏽', emojiAvatarColors[11]);
    final notifier = _FakeProfileNotifier(
      profile: UserProfile(pubkey: 'aabb', avatarUrl: avatarUrl),
    );
    await tester.pumpWidget(
      WidgetHelpers.testable(
        overrides: [
          profileProvider.overrideWith(() => notifier),
          emojiDatasetOrEmptyProvider.overrideWithValue(dataset),
        ],
        child: const ProfileEditPage(),
      ),
    );
    await tester.pumpAndSettle();

    await tester.tap(find.text('Edit Photo'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Emoji'));
    await tester.pump(const Duration(milliseconds: 250));

    expect(
      tester
          .widget<PopupMenuButton<int>>(
            find.byKey(const ValueKey('emoji-avatar-skin-tone')),
          )
          .initialValue,
      3,
    );
  });

  testWidgets('discards a delayed image after switching avatar modes', (
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

    await tester.tap(find.text('Emoji'));
    await tester.pump(const Duration(milliseconds: 250));
    uploadService.completeGallerySelection();
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 300));

    expect(find.text('Position Photo'), findsNothing);
    expect(find.byKey(const ValueKey('emoji-avatar-preview')), findsOneWidget);
  });
}
