part of '../profile_edit_page_test.dart';

void runProfileEditImageSelectionTests() {
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
