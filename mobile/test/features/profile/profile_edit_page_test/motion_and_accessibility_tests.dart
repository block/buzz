part of '../profile_edit_page_test.dart';

void runProfileEditMotionAndAccessibilityTests() {
  testWidgets('photo modes remain usable on a compact large-type viewport', (
    tester,
  ) async {
    tester.view.devicePixelRatio = 1;
    tester.view.physicalSize = const Size(320, 568);
    addTearDown(tester.view.resetDevicePixelRatio);
    addTearDown(tester.view.resetPhysicalSize);
    await tester.pumpWidget(
      WidgetHelpers.testable(
        overrides: [profileProvider.overrideWith(_FakeProfileNotifier.new)],
        child: const MediaQuery(
          data: MediaQueryData(textScaler: TextScaler.linear(2)),
          child: ProfileEditPage(startInPhotoEditor: true),
        ),
      ),
    );
    await tester.pump(const Duration(milliseconds: 250));
    expect(tester.takeException(), isNull);
    expect(
      find.byKey(const ValueKey('avatar-editor-scroll-view')),
      findsOneWidget,
    );

    await tester.drag(
      find.byKey(const ValueKey('avatar-editor-scroll-view')),
      const Offset(0, 1000),
    );
    await tester.pump();
    final animatedMode = find.byKey(const ValueKey('avatar-mode-animated'));
    await Scrollable.ensureVisible(
      animatedMode.evaluate().single,
      alignment: 0.2,
    );
    await tester.tap(animatedMode);
    await tester.pump(const Duration(milliseconds: 250));
    expect(tester.takeException(), isNull);
    expect(
      find.byKey(const ValueKey('animated-avatar-capture-preview')),
      findsOneWidget,
    );
  });

  testWidgets('exposes the selected avatar mode on Android', (tester) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.android;
    addTearDown(() => debugDefaultTargetPlatformOverride = null);
    await tester.pumpWidget(
      WidgetHelpers.testable(
        overrides: [profileProvider.overrideWith(_FakeProfileNotifier.new)],
        child: const ProfileEditPage(),
      ),
    );
    await tester.pumpAndSettle();
    await tester.tap(find.text('Edit Photo'));
    await tester.pumpAndSettle();

    Iterable<Semantics> modeSemantics(String label) =>
        tester.widgetList<Semantics>(
          find.byWidgetPredicate(
            (widget) =>
                widget is Semantics &&
                widget.properties.label == label &&
                widget.child is ExcludeSemantics,
          ),
        );

    expect(modeSemantics('Image'), isNotEmpty);
    expect(
      modeSemantics('Image').every((node) => node.properties.selected == true),
      isTrue,
    );
    expect(
      modeSemantics(
        'Animated',
      ).every((node) => node.properties.selected == false),
      isTrue,
    );
    await tester.tap(find.byKey(const ValueKey('avatar-mode-animated')));
    await tester.pump();
    expect(
      modeSemantics('Image').every((node) => node.properties.selected == false),
      isTrue,
    );
    expect(
      modeSemantics(
        'Animated',
      ).every((node) => node.properties.selected == true),
      isTrue,
    );
    debugDefaultTargetPlatformOverride = null;
  });

  testWidgets('image content returns from animated mode', (tester) async {
    await tester.pumpWidget(
      WidgetHelpers.testable(
        overrides: [profileProvider.overrideWith(_FakeProfileNotifier.new)],
        child: const ProfileEditPage(startInPhotoEditor: true),
      ),
    );
    await tester.pumpAndSettle();
    await tester.tap(find.text('Animated'));
    await tester.pump(const Duration(milliseconds: 250));
    expect(
      find.byKey(const ValueKey('animated-avatar-capture-preview')),
      findsOneWidget,
    );
    await tester.tap(find.text('Image'));
    await tester.pump();
    final transform = tester.widget<Transform>(
      find.byKey(const ValueKey('avatar-mode-transition-transform')),
    );
    expect(transform.transform.getTranslation().x, lessThan(0));
    await tester.pump(const Duration(milliseconds: 250));
    expect(
      find.byKey(const ValueKey('avatar-editor-fixed-preview')),
      findsOneWidget,
    );
  });

  testWidgets('retains the preview while animated mode initializes', (
    tester,
  ) async {
    await tester.pumpWidget(
      WidgetHelpers.testable(
        overrides: [profileProvider.overrideWith(_FakeProfileNotifier.new)],
        child: const ProfileEditPage(startInPhotoEditor: true),
      ),
    );
    await tester.pumpAndSettle();
    final imageCenter = tester
        .getCenter(find.byKey(const ValueKey('avatar-editor-fixed-preview')))
        .dy;
    await tester.tap(find.text('Animated'));
    await tester.pump();
    final retained = find.byKey(const ValueKey('avatar-mode-retained-preview'));
    expect(retained, findsOneWidget);
    expect(tester.getCenter(retained).dy, closeTo(imageCenter, 0.01));
    await tester.pump(const Duration(milliseconds: 150));
    expect(retained, findsNothing);
  });

  testWidgets('plays an animated avatar on the profile and image editor', (
    tester,
  ) async {
    const avatar =
        'https://relay.example/poster.png#buzz-anim=https%3A%2F%2Frelay.example%2Fanimation.png';
    await tester.pumpWidget(
      WidgetHelpers.testable(
        overrides: [
          profileProvider.overrideWith(
            () => _FakeProfileNotifier(
              profile: const UserProfile(
                pubkey: 'aabb',
                displayName: 'Alice',
                about: 'Building Buzz',
                avatarUrl: avatar,
              ),
            ),
          ),
        ],
        child: const ProfileEditPage(),
      ),
    );
    await tester.pump();

    expect(find.byType(PlayingAvatarImage), findsOneWidget);
    expect(find.byType(ProgressiveAnimatedAvatar), findsOneWidget);

    await tester.tap(find.text('Edit Photo'));
    await tester.pump();
    expect(find.byType(PlayingAvatarImage), findsOneWidget);
    expect(find.byType(ProgressiveAnimatedAvatar), findsOneWidget);
  });

  testWidgets('shows only the animated-avatar poster with Reduce Motion', (
    tester,
  ) async {
    const avatar =
        'https://relay.example/poster.png#buzz-anim=https%3A%2F%2Frelay.example%2Fanimation.png';
    await tester.pumpWidget(
      WidgetHelpers.testable(
        overrides: [
          profileProvider.overrideWith(
            () => _FakeProfileNotifier(
              profile: const UserProfile(
                pubkey: 'aabb',
                displayName: 'Alice',
                avatarUrl: avatar,
              ),
            ),
          ),
        ],
        child: const MediaQuery(
          data: MediaQueryData(disableAnimations: true),
          child: ProfileEditPage(),
        ),
      ),
    );
    await tester.pump();

    expect(find.byType(ProgressiveAnimatedAvatar), findsNothing);
    expect(
      tester.widget<AvatarImage>(find.byType(AvatarImage)).imageUrl,
      'https://relay.example/poster.png',
    );
  });

  testWidgets('uses high-contrast inverse colors for avatar action icons', (
    tester,
  ) async {
    final theme = AppTheme.dark();
    await tester.pumpWidget(
      MaterialApp(
        theme: theme,
        home: Scaffold(
          body: Row(
            children: [
              Expanded(
                child: AvatarEditorOptionButton(
                  icon: Icons.palette,
                  label: 'Inactive',
                  selected: false,
                  onTap: () {},
                ),
              ),
              Expanded(
                child: AvatarEditorOptionButton(
                  icon: Icons.face,
                  label: 'Active',
                  selected: true,
                  onTap: () {},
                ),
              ),
            ],
          ),
        ),
      ),
    );

    expect(
      tester.widget<Icon>(find.byIcon(Icons.palette)).color,
      theme.colorScheme.onSurface,
    );
    expect(
      tester.widget<Icon>(find.byIcon(Icons.face)).color,
      theme.colorScheme.surface,
    );
    final selectedSemantics = tester.widget<Semantics>(
      find.byWidgetPredicate(
        (widget) => widget is Semantics && widget.properties.label == 'Active',
      ),
    );
    expect(selectedSemantics.properties.button, isTrue);
    expect(selectedSemantics.properties.selected, isTrue);
  });

  testWidgets('uses liquid-glass avatar rail icons on iOS', (tester) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
    addTearDown(() => debugDefaultTargetPlatformOverride = null);
    await tester.pumpWidget(
      WidgetHelpers.testable(
        child: Row(
          children: [
            Expanded(
              child: AvatarEditorOptionButton(
                icon: Icons.palette,
                iosIcon: IosGlassNavigationIcon.palette,
                label: 'Background',
                selected: true,
                onTap: () {},
              ),
            ),
            Expanded(
              child: AvatarEditorOptionButton(
                icon: Icons.face,
                iosIcon: IosGlassNavigationIcon.emoji,
                label: 'Emoji',
                selected: false,
                onTap: () {},
              ),
            ),
          ],
        ),
      ),
    );

    final nativeIcons = tester
        .widgetList<UiKitView>(find.byType(UiKitView))
        .map((view) => view.creationParams as Map<String, Object>)
        .map((params) => params['icon'])
        .toList();
    expect(nativeIcons, containsAll(['palette', 'emoji']));
    final selectedControls = tester
        .widgetList<UiKitView>(find.byType(UiKitView))
        .map((view) => view.creationParams as Map<String, Object>)
        .where((params) => params['selected'] == true)
        .toList();
    expect(selectedControls.single['icon'], 'palette');
    expect(
      selectedControls.single['foregroundColor'],
      AppTheme.light().colorScheme.primary.toARGB32(),
    );
    debugDefaultTargetPlatformOverride = null;
  });

  testWidgets('doubles the avatar rail icon-to-label spacing', (tester) async {
    await tester.pumpWidget(
      WidgetHelpers.testable(
        child: AvatarEditorOptionButton(
          icon: Icons.palette,
          label: 'Background',
          selected: false,
          onTap: () {},
        ),
      ),
    );

    final surface = find.byType(AnimatedContainer);
    expect(
      tester.getRect(find.text('Background')).top -
          tester.getRect(surface).bottom,
      avatarEditorOptionLabelGap,
    );
  });

  testWidgets('uses liquid glass for animated capture and review on iOS', (
    tester,
  ) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
    addTearDown(() => debugDefaultTargetPlatformOverride = null);
    await tester.pumpWidget(
      WidgetHelpers.testable(
        child: SizedBox(
          height: 500,
          child: AnimatedAvatarCapture(
            key: ValueKey('animated-capture-glass-test'),
            height: 500,
            onPrepareChanged: (_) {},
          ),
        ),
      ),
    );
    await tester.pump();

    var nativeControls = tester
        .widgetList<UiKitView>(find.byType(UiKitView))
        .map((view) => view.creationParams as Map<String, Object>)
        .toList();
    expect(
      nativeControls.any(
        (params) => params['icon'] == 'shutter' && params['label'] == 'Record',
      ),
      isTrue,
    );

    final frame = Uint8List.fromList(
      image.encodePng(image.Image(width: 8, height: 8)),
    );
    await tester.pumpWidget(
      WidgetHelpers.testable(
        child: SizedBox(
          height: 500,
          child: AnimatedAvatarCapture(
            key: ValueKey('animated-review-glass-test'),
            height: 500,
            initialFrames: [frame, frame],
            onPrepareChanged: (_) {},
          ),
        ),
      ),
    );
    await tester.pump();

    nativeControls = tester
        .widgetList<UiKitView>(find.byType(UiKitView))
        .map((view) => view.creationParams as Map<String, Object>)
        .toList();
    final reviewIcons = nativeControls
        .map((params) => params['icon'])
        .whereType<String>()
        .toList();
    expect(reviewIcons, containsAll(['person', 'palette', 'frame', 'camera']));
    debugDefaultTargetPlatformOverride = null;
  });

  testWidgets('background colors remain reachable in compact layouts', (
    tester,
  ) async {
    await tester.pumpWidget(
      WidgetHelpers.testable(
        child: SizedBox(
          height: 150,
          child: AvatarBackgroundGrid(
            selectedColor: emojiAvatarColors.first,
            onColorSelected: (_) {},
          ),
        ),
      ),
    );

    final scrollable = tester.state<ScrollableState>(
      find.byType(Scrollable).first,
    );
    expect(scrollable.position.maxScrollExtent, greaterThan(0));

    await tester.drag(find.byType(AvatarBackgroundGrid), const Offset(0, -400));
    await tester.pumpAndSettle();

    expect(scrollable.position.pixels, greaterThan(0));
    expect(
      find.byKey(
        ValueKey('avatar-background-color-${emojiAvatarColors.length - 1}'),
      ),
      findsOneWidget,
    );
  });
}
