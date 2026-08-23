part of '../profile_edit_page_test.dart';

void runProfileEditImageSelectionTests() {
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
