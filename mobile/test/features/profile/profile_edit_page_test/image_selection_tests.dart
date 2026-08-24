part of '../profile_edit_page_test.dart';

void runProfileEditImageSelectionTests() {
  testWidgets('grows the inline camera viewfinder by 25dp', (tester) async {
    await tester.pumpWidget(
      WidgetHelpers.testable(
        child: Scaffold(
          body: SizedBox(
            height: 400,
            child: ImageAvatarCapture(
              height: 400,
              onAccepted: (_) {},
              onClosed: () {},
              loadCameras: () async => const [],
            ),
          ),
        ),
      ),
    );

    final preview = find.byKey(const ValueKey('image-camera-preview-size'));
    expect(tester.getSize(preview), const Size.square(220));
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 180));
    expect(tester.getSize(preview), const Size.square(245));
    expect(find.bySemanticsLabel('Close camera'), findsOneWidget);
    expect(find.bySemanticsLabel('Flip camera'), findsOneWidget);
    expect(find.bySemanticsLabel('Take photo'), findsOneWidget);
  });

  testWidgets('shrinks a captured photo and accepts it with a check', (
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
    await tester.pumpAndSettle();

    expect(
      tester.getSize(find.byKey(const ValueKey('image-camera-preview-size'))),
      const Size.square(220),
    );
    expect(
      tester.getSize(find.byKey(const ValueKey('image-camera-shutter-morph'))),
      const Size.square(64),
    );
    expect(
      find.byKey(const ValueKey('image-camera-accept-icon')),
      findsOneWidget,
    );
    expect(find.bySemanticsLabel('Retake photo'), findsOneWidget);

    await tester.tap(find.byKey(const ValueKey('image-camera-shutter')));
    expect(accepted, same(bytes));
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
