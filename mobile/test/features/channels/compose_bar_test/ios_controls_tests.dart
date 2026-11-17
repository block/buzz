part of '../compose_bar_test.dart';

class _TrayPickerService extends _FakeVoiceNoteUploadService {
  int photoOpens = 0;
  @override
  Future<List<XFile>> pickGalleryImages() async {
    photoOpens++;
    return [];
  }
}

void iosControlsTests() {
  setUp(() {
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(
          const MethodChannel('buzz/native_attachment_popover'),
          (call) async => false,
        );
  });
  tearDown(() {
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(
          const MethodChannel('buzz/native_attachment_popover'),
          null,
        );
  });
  testWidgets('inactive and active empty iOS composer use a capsule', (
    tester,
  ) async {
    tester.view.devicePixelRatio = 1;
    tester.view.physicalSize = const Size(320, 700);
    addTearDown(tester.view.reset);
    const hint = 'Message #a-channel-with-a-very-long-name-that-must-not-wrap';
    final recorder = _FakeVoiceNoteRecorder();
    await tester.pumpWidget(
      _buildComposeBar(
        hintText: hint,
        voiceNoteRecorderFactory: () => recorder,
        uploadService: _TrayPickerService(),
        onSend: (_, _, {mediaTags = const []}) async {},
      ),
    );
    final surface = find.byKey(
      const ValueKey('composer-ios-concentric-surface'),
    );
    expect(tester.widget<ConcentricSheetSurface>(surface).capsule, isTrue);
    await _tapComposerControl(tester, 'Attachments');
    await tester.pumpAndSettle();
    expect(
      find.byKey(const ValueKey('attachment-menu-item-photos')),
      findsOneWidget,
    );
    expect(find.byType(TextField), findsNothing);
    await tester.tapAt(const Offset(10, 10));
    await tester.pumpAndSettle();
    expect(find.byTooltip('Photos'), findsNothing);
    await tester.tap(find.text(hint));
    await tester.pumpAndSettle();
    expect(tester.widget<ConcentricSheetSurface>(surface).capsule, isTrue);
    expect(tester.getSize(find.text(hint)).height, lessThan(30));
    final plus = find.byKey(const ValueKey('ios-composer-plus'));
    final composer = find.byKey(const ValueKey('composer-surface'));
    expect(tester.getSize(plus).height, tester.getSize(composer).height);
    expect(tester.getTopLeft(composer).dx - tester.getTopRight(plus).dx, 8);
    expect(find.byTooltip('Send'), findsNothing);
    expect(find.byTooltip('Emoji'), findsOneWidget);
    expect(find.byTooltip('Voice note'), findsOneWidget);
    final emptyComposerSize = tester.getSize(composer);
    await tester.enterText(find.byType(TextField), 'A draft');
    await tester.pumpAndSettle();
    expect(tester.widget<ConcentricSheetSurface>(surface).capsule, isFalse);
    expect(tester.getSize(composer).width, emptyComposerSize.width);
    expect(find.byTooltip('Send'), findsOneWidget);
    expect(find.byTooltip('Emoji'), findsOneWidget);
    expect(find.byTooltip('Voice note'), findsNothing);
    await tester.enterText(find.byType(TextField), '');
    await tester.pumpAndSettle();
    expect(find.byTooltip('Send'), findsNothing);
    expect(find.byTooltip('Emoji'), findsOneWidget);
    expect(find.byTooltip('Voice note'), findsOneWidget);
    await _tapComposerControl(tester, 'Voice note');
    await tester.pumpAndSettle();
    expect(recorder.started, isTrue);
    expect(find.byKey(const ValueKey('voice-note-recorder')), findsOneWidget);
    await tester.tap(find.byKey(const ValueKey('voice-note-recorder-close')));
    await tester.pumpAndSettle();
    expect(recorder.cancelled, isTrue);
    expect(find.byKey(const ValueKey('ios-composer-plus')), findsOneWidget);
    await tester.pumpWidget(const SizedBox.shrink());
  }, variant: TargetPlatformVariant.only(TargetPlatform.iOS));

  testWidgets('selected text opens native Format after edit menu dismissal', (
    tester,
  ) async {
    final menus = _captureNativeComposerMenus();
    await tester.pumpWidget(
      _buildComposeBar(
        uploadService: _TrayPickerService(),
        supportsShowingSystemContextMenu: true,
        onSend: (_, _, {mediaTags = const []}) async {},
      ),
    );
    await _expandComposer(tester);
    final editable = tester.state<EditableTextState>(find.byType(EditableText));
    final controller = tester
        .widget<TextField>(find.byType(TextField))
        .controller!;
    for (final style in [
      ('Bold', '**word**'),
      ('Italic', '_word_'),
      ('Strikethrough', '~~word~~'),
      ('Code', '`word`'),
      ('Code block', '```\nword\n```'),
    ]) {
      controller.value = const TextEditingValue(
        text: 'word',
        selection: TextSelection(baseOffset: 0, extentOffset: 4),
      );
      await tester.pump();
      editable.showToolbar();
      await tester.pumpAndSettle();
      expect(menus.last.first['title'], 'Format');
      final format = menus.last.singleWhere(
        (item) => item['title'] == 'Format',
      );
      await _nativeComposerMenuEvent('ContextMenu.onPerformCustomAction', [
        null,
        format['id'] as String,
      ]);
      await _nativeComposerMenuEvent('ContextMenu.onDismissSystemContextMenu');
      await tester.pumpAndSettle();
      expect(menus.last.map((item) => item['title']), [
        'Bold',
        'Italic',
        'Strikethrough',
        'Code',
        'Code block',
      ]);
      final action = menus.last.singleWhere(
        (item) => item['title'] == style.$1,
      );
      await _nativeComposerMenuEvent('ContextMenu.onPerformCustomAction', [
        null,
        action['id'] as String,
      ]);
      await tester.pumpAndSettle();
      expect(controller.text, style.$2);
      expect(
        tester.widget<TextField>(find.byType(TextField)).focusNode!.hasFocus,
        isTrue,
      );
    }
    controller.selection = TextSelection.collapsed(
      offset: controller.text.length,
    );
    editable.showToolbar();
    await tester.pumpAndSettle();
    expect(menus.last.where((item) => item['title'] == 'Format'), isEmpty);
    await tester.pumpWidget(const SizedBox.shrink());
  }, variant: TargetPlatformVariant.only(TargetPlatform.iOS));

  testWidgets('attachment popover stays in its resized chat pane', (
    tester,
  ) async {
    tester.view.devicePixelRatio = 1;
    tester.view.physicalSize = const Size(1200, 900);
    tester.view.viewInsets = const FakeViewPadding(bottom: 300);
    addTearDown(tester.view.reset);
    final paneWidth = ValueNotifier(800.0);
    addTearDown(paneWidth.dispose);
    await tester.pumpWidget(
      _buildComposeBar(
        uploadService: _TrayPickerService(),
        onSend: (_, _, {mediaTags = const []}) async {},
        wrapPage: (page) => ValueListenableBuilder<double>(
          valueListenable: paneWidth,
          builder: (_, width, _) => Align(
            alignment: Alignment.centerRight,
            child: SizedBox(width: width, child: page),
          ),
        ),
      ),
    );
    await tester.tap(find.text('Message'));
    await tester.pumpAndSettle();
    final plus = find.byKey(const ValueKey('ios-composer-plus'));
    final glass = find.byWidgetPredicate(
      (widget) =>
          widget is UiKitView &&
          widget.viewType == 'buzz/composer_tool_strip_glass',
    );
    for (final width in [800.0, 500.0]) {
      paneWidth.value = width;
      await tester.pumpAndSettle();
      expect(tester.getRect(plus).left, 1200 - width + 12);
      await _tapComposerControl(tester, 'Attachments');
      await tester.pumpAndSettle();
      expect(glass, findsOneWidget);
      expect(tester.getRect(plus).right, lessThanOrEqualTo(1188));
      await tester.tap(find.byType(TextField));
      await tester.pumpAndSettle();
    }
    await tester.pumpWidget(const SizedBox.shrink());
  }, variant: TargetPlatformVariant.only(TargetPlatform.iOS));

  testWidgets(
    'iOS attachments and empty composer actions work without a keyboard',
    (tester) async {
      tester.view.devicePixelRatio = 1;
      tester.view.physicalSize = const Size(800, 900);
      tester.view.viewPadding = const FakeViewPadding(bottom: 34);
      tester.view.padding = const FakeViewPadding(bottom: 34);
      addTearDown(tester.view.reset);
      final pickerService = _TrayPickerService();
      const photoChannel = MethodChannel('buzz/inline_photo_picker');
      tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
        photoChannel,
        (call) async => true,
      );
      addTearDown(
        () => tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
          photoChannel,
          null,
        ),
      );
      const emojiChannel = MethodChannel('buzz/native_emoji_picker');
      final nativeCalls = <String>[];
      tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
        emojiChannel,
        (call) async {
          nativeCalls.add(call.method);
          return true;
        },
      );
      addTearDown(
        () => tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
          emojiChannel,
          null,
        ),
      );
      await tester.pumpWidget(
        _buildComposeBar(
          uploadService: pickerService,
          customEmoji: const [
            CustomEmoji(
              shortcode: 'party',
              url: 'https://example.org/party.png',
            ),
          ],
          onSend: (content, mentions, {mediaTags = const []}) async {},
        ),
      );
      expect(find.byTooltip('Add attachment'), findsOneWidget);
      await tester.tap(find.text('Message'));
      await tester.pumpAndSettle();
      for (final label in ['Emoji', 'Voice note']) {
        expect(find.byTooltip(label), findsOneWidget);
      }
      expect(find.byKey(const ValueKey('standard-actions')), findsNothing);
      expect(find.byTooltip('Add attachment'), findsOneWidget);
      await _tapComposerControl(tester, 'Attachments');
      await tester.pumpAndSettle();
      for (final label in ['camera', 'photos', 'voice note', 'files']) {
        expect(
          find.byKey(ValueKey('attachment-menu-item-$label')),
          findsOneWidget,
        );
      }
      expect(find.text('Dictate'), findsNothing);
      await tester.tap(
        find.byKey(const ValueKey('attachment-menu-item-camera')),
      );
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 200));
      expect(find.byKey(const ValueKey('camera-preview')), findsOneWidget);
      await tester.tap(find.byType(TextField));
      await tester.pumpAndSettle();
      await _selectComposerAttachment(tester, 'Files');
      await _selectComposerAttachment(tester, 'Photos');
      expect(pickerService.photoOpens, 0);
      expect(find.byKey(const ValueKey('photo-gallery')), findsOneWidget);
      expect(find.byKey(const ValueKey('attachment-menu')), findsNothing);
      expect(
        find.byKey(const ValueKey('ios-inline-photo-picker')),
        findsOneWidget,
      );
      expect(find.byTooltip('Keyboard'), findsNothing);
      await tester.tapAt(const Offset(20, 100));
      await tester.pumpAndSettle();
      await tester.tap(find.byTooltip('Emoji'));
      await tester.pumpAndSettle();
      expect(nativeCalls, contains('present'));
      expect(tester.testTextInput.isVisible, isFalse);
      for (var index = 0; index < 2; index++) {
        await tester.binding.defaultBinaryMessenger.handlePlatformMessage(
          emojiChannel.name,
          emojiChannel.codec.encodeMethodCall(
            const MethodCall('selected', ':party:'),
          ),
          (_) {},
        );
        await tester.pumpAndSettle();
        expect(tester.testTextInput.isVisible, isFalse);
      }
      expect(
        tester.widget<TextField>(find.byType(TextField).first).controller!.text,
        ':party::party:',
      );
      expect(find.byType(EmojiPickerSheet), findsNothing);
      await tester.binding.defaultBinaryMessenger.handlePlatformMessage(
        emojiChannel.name,
        emojiChannel.codec.encodeMethodCall(const MethodCall('dismissed')),
        (_) {},
      );
      await tester.pumpAndSettle();
      expect(tester.testTextInput.isVisible, isTrue);
      await tester.pumpWidget(const SizedBox.shrink());
    },
    variant: TargetPlatformVariant.only(TargetPlatform.iOS),
  );

  testWidgets(
    'tapping the draft with the attachment menu open goes back to typing',
    (tester) async {
      await tester.pumpWidget(
        _buildComposeBar(
          uploadService: _TrayPickerService(),
          onSend: (_, _, {mediaTags = const []}) async {},
        ),
      );
      await _tapComposerControl(tester, 'Attachments');
      await tester.pumpAndSettle();
      expect(find.byKey(const ValueKey('attachment-menu')), findsOneWidget);
      await tester.tap(find.text('Message'));
      await tester.pumpAndSettle();
      expect(find.byKey(const ValueKey('attachment-menu')), findsNothing);
      expect(find.byType(TextField), findsOneWidget);
      await tester.pumpWidget(const SizedBox.shrink());
    },
    variant: TargetPlatformVariant.only(TargetPlatform.iOS),
  );

  testWidgets('choosing photos without the native popover closes the tray', (
    tester,
  ) async {
    tester.view.devicePixelRatio = 1;
    tester.view.physicalSize = const Size(800, 900);
    addTearDown(tester.view.reset);
    const photoChannel = MethodChannel('buzz/inline_photo_picker');
    tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
      photoChannel,
      (call) async => false,
    );
    addTearDown(
      () => tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
        photoChannel,
        null,
      ),
    );
    Future<void> settle() async {
      for (var i = 0; i < 20; i++) {
        await tester.pump(const Duration(milliseconds: 100));
      }
    }

    await tester.pumpWidget(
      _buildComposeBar(
        uploadService: _testUploadService(nostr.Keys.generate().nsec),
        photoLibrary: _FakePhotoLibrary([
          RecentPhoto(id: 'one', thumbnailBytes: _gifBytes),
        ]),
        onSend: (content, mentions, {mediaTags = const []}) async {},
      ),
    );
    await _tapComposerControl(tester, 'Attachments');
    await settle();
    expect(find.byKey(const ValueKey('attachment-menu')), findsOneWidget);
    await tester.tap(find.byKey(const ValueKey('attachment-menu-item-photos')));
    await settle();
    await tester.tap(find.byKey(const ValueKey('recent-photo-one')));
    await settle();
    await tester.tap(find.text('Add 1 photo'));
    await settle();
    expect(
      find.byWidgetPredicate(
        (w) => w.runtimeType.toString() == '_ComposerToolStripGlass',
      ),
      findsNothing,
    );
    await tester.pumpWidget(const SizedBox.shrink());
  }, variant: TargetPlatformVariant.only(TargetPlatform.iOS));

  testWidgets('typed mentions and channels preserve the draft and keyboard', (
    tester,
  ) async {
    tester.view.devicePixelRatio = 1;
    tester.view.physicalSize = const Size(800, 900);
    tester.view.viewPadding = const FakeViewPadding(bottom: 34);
    tester.view.viewInsets = const FakeViewPadding(bottom: 300);
    addTearDown(tester.view.reset);
    await tester.pumpWidget(
      _buildComposeBar(
        channels: [_makeCurrentChannel(), _makeSharedMemberChannel()],
        members: [
          ChannelMember(
            pubkey: 'a' * 64,
            displayName: 'Scout',
            role: 'member',
            joinedAt: DateTime(2025),
          ),
        ],
        uploadService: _testUploadService(nostr.Keys.generate().nsec),
        onSend: (content, mentions, {mediaTags = const []}) async {},
      ),
    );
    await tester.tap(find.text('Message'));
    await tester.pumpAndSettle();
    for (final action in [
      ('Mention', 'mention-suggestions-popover'),
      ('Channel', 'channel-suggestions-popover'),
    ]) {
      await tester.enterText(find.byType(TextField), '');
      await tester.pumpAndSettle();
      tester.testTextInput.log.clear();
      await tester.enterText(
        find.byType(TextField),
        action.$1 == 'Mention' ? '@' : '#',
      );
      await tester.pumpAndSettle();
      expect(tester.testTextInput.isVisible, isTrue);
      expect(
        tester.testTextInput.log.where(
          (call) => call.method == 'TextInput.hide',
        ),
        isEmpty,
      );
      expect(
        tester.widget<TextField>(find.byType(TextField)).controller!.text,
        action.$1 == 'Mention' ? '@' : '#',
      );
      expect(find.byKey(ValueKey(action.$2)), findsOneWidget);
      expect(find.byKey(const ValueKey('ios-composer-plus')), findsOneWidget);
      await tester.tapAt(const Offset(20, 100));
      await tester.pumpAndSettle();
      expect(find.byKey(ValueKey(action.$2)), findsNothing);
      expect(find.byKey(const ValueKey('ios-composer-plus')), findsOneWidget);
      await tester.enterText(find.byType(TextField), '');
      await tester.pumpAndSettle();
      await tester.enterText(
        find.byType(TextField),
        action.$1 == 'Mention' ? '@' : '#',
      );
      await tester.pumpAndSettle();
      await tester.tap(find.text(action.$1 == 'Mention' ? 'Scout' : 'shared'));
      await tester.pumpAndSettle();
      expect(find.byKey(ValueKey(action.$2)), findsNothing);
      expect(find.byKey(const ValueKey('ios-composer-plus')), findsOneWidget);
      expect(tester.testTextInput.isVisible, isTrue);
      expect(
        tester.widget<TextField>(find.byType(TextField)).controller!.text,
        action.$1 == 'Mention' ? '@Scout ' : '#shared ',
      );
    }
    // A real tap on Emoji while autocomplete is open opens the picker
    // instead of only dismissing the suggestions.
    const emojiChannel = MethodChannel('buzz/native_emoji_picker');
    var emojiPresentations = 0;
    tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
      emojiChannel,
      (call) async {
        if (call.method == 'present') emojiPresentations++;
        return true;
      },
    );
    addTearDown(
      () => tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
        emojiChannel,
        null,
      ),
    );
    await tester.enterText(find.byType(TextField), '@');
    await tester.pumpAndSettle();
    expect(
      find.byKey(const ValueKey('mention-suggestions-popover')),
      findsOneWidget,
    );
    await tester.tap(find.byTooltip('Emoji'));
    await tester.pumpAndSettle();
    expect(emojiPresentations, 1);
    await tester.binding.defaultBinaryMessenger.handlePlatformMessage(
      emojiChannel.name,
      emojiChannel.codec.encodeMethodCall(const MethodCall('dismissed')),
      (_) {},
    );
    await tester.pumpAndSettle();
    await tester.enterText(find.byType(TextField), 'Keep this draft');
    await tester.pumpAndSettle();
    final originalDraftTop = tester.getTopLeft(find.byType(TextField)).dy;
    await _tapComposerControl(tester, 'Attachments');
    await tester.pumpAndSettle();
    // This fixture declines native presentation, so the inline fallback
    // replaces the keyboard while preserving the draft and its position.
    expect(tester.testTextInput.isVisible, isFalse);
    expect(tester.getTopLeft(find.byType(TextField)).dy, originalDraftTop);
    await tester.tapAt(const Offset(20, 100));
    await tester.pumpAndSettle();
    expect(
      tester.widget<TextField>(find.byType(TextField)).controller!.text,
      'Keep this draft',
    );
    expect(find.byKey(const ValueKey('attachment-surface')), findsNothing);
    await tester.pumpWidget(const SizedBox.shrink());
  }, variant: TargetPlatformVariant.only(TargetPlatform.iOS));

  testWidgets('a Send tap is not taken as a tap outside the suggestions', (
    tester,
  ) async {
    await tester.pumpWidget(
      _buildComposeBar(
        channels: [_makeCurrentChannel(), _makeSharedMemberChannel()],
        members: [
          ChannelMember(
            pubkey: 'a' * 64,
            displayName: 'Scout',
            role: 'member',
            joinedAt: DateTime(2025),
          ),
        ],
        uploadService: _testUploadService(nostr.Keys.generate().nsec),
        onSend: (content, mentions, {mediaTags = const []}) async {},
      ),
    );
    await _expandComposer(tester);
    await tester.enterText(find.byType(TextField), 'hi @Sc');
    await tester.pumpAndSettle();
    final suggestions = find.byKey(
      const ValueKey('mention-suggestions-popover'),
    );
    expect(suggestions, findsOneWidget);
    // Send is a native view, so a widget test cannot press it. A tap the
    // tray consumes as "outside" dismisses the suggestions and never reaches
    // the native button; a tap that reaches it leaves them in place.
    await tester.tap(find.byTooltip('Send'));
    await tester.pumpAndSettle();
    expect(suggestions, findsOneWidget);
    await tester.pumpWidget(const SizedBox.shrink());
  }, variant: TargetPlatformVariant.only(TargetPlatform.iOS));
}
