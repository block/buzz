part of '../profile_edit_page_test.dart';

void runProfileEditMotionAndAccessibilityTests() {
  testWidgets('moves segment content in the selected direction', (
    tester,
  ) async {
    await tester.pumpWidget(
      WidgetHelpers.testable(
        overrides: [profileProvider.overrideWith(_FakeProfileNotifier.new)],
        child: const ProfileEditPage(),
      ),
    );
    await tester.pumpAndSettle();
    await tester.tap(find.text('Edit Photo'));
    await tester.pumpAndSettle();

    await tester.tap(find.text('Emoji'));
    await tester.pump();
    final forwardTransform = tester.widget<Transform>(
      find.byKey(const ValueKey('avatar-mode-transition-transform')),
    );
    expect(forwardTransform.transform.getTranslation().x, greaterThan(0));
    await tester.pump(const Duration(milliseconds: 240));
    expect(
      tester
          .widget<Transform>(
            find.byKey(const ValueKey('avatar-mode-transition-transform')),
          )
          .transform
          .getTranslation()
          .x,
      closeTo(0, 0.01),
    );

    await tester.tap(find.text('Image'));
    await tester.pump();
    final reverseTransform = tester.widget<Transform>(
      find.byKey(const ValueKey('avatar-mode-transition-transform')),
    );
    expect(reverseTransform.transform.getTranslation().x, lessThan(0));
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

  testWidgets('keeps emoji actions anchored when search opens the keyboard', (
    tester,
  ) async {
    tester.view.devicePixelRatio = 1;
    tester.view.physicalSize = const Size(800, 900);
    addTearDown(tester.view.reset);
    await tester.pumpWidget(
      WidgetHelpers.testable(
        overrides: [profileProvider.overrideWith(_FakeProfileNotifier.new)],
        child: const ProfileEditPage(),
      ),
    );
    await tester.pumpAndSettle();
    await tester.tap(find.text('Edit Photo'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Emoji'));
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 150));

    final action = find.byKey(const ValueKey('emoji-editor-background'));
    final actionBottomBefore = tester.getRect(action).bottom;
    await tester.tap(find.byKey(const ValueKey('emoji-avatar-search')));
    tester.view.viewInsets = const FakeViewPadding(bottom: 300);
    await tester.pump();

    expect(tester.getRect(action).bottom, actionBottomBefore);
    expect(
      tester.getRect(find.byKey(const ValueKey('emoji-avatar-search'))).bottom,
      lessThan(600),
    );
    expect(
      tester
          .widgetList<Scaffold>(find.byType(Scaffold))
          .any((scaffold) => scaffold.resizeToAvoidBottomInset == false),
      isTrue,
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

  testWidgets('uses the shared animated background grid for emoji avatars', (
    tester,
  ) async {
    await tester.pumpWidget(
      WidgetHelpers.testable(
        overrides: [profileProvider.overrideWith(_FakeProfileNotifier.new)],
        child: const ProfileEditPage(),
      ),
    );
    await tester.pump();
    await tester.tap(find.text('Edit Photo'));
    await tester.pump(const Duration(milliseconds: 250));
    await tester.tap(find.text('Emoji'));
    await tester.pump(const Duration(milliseconds: 250));
    await tester.tap(find.byKey(const ValueKey('emoji-editor-background')));
    await tester.pump(const Duration(milliseconds: 200));

    expect(find.byType(AvatarBackgroundGrid), findsOneWidget);
    final firstColor = find.byKey(const ValueKey('emoji-avatar-color-0'));
    expect(tester.getSize(firstColor), const Size.square(52));
  });
}
