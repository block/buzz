part of '../channels_page_test.dart';

void presenceListTests(
  Widget Function(List<Override>) harness,
  List<Channel> channels,
) {
  testWidgets('DM list hides unconfirmed dots and paints confirmed presence', (
    tester,
  ) async {
    final cache = _ListPresenceFixture();
    await tester.pumpWidget(
      harness([
        channelsProvider.overrideWith(() => _FakeNotifier(channels)),
        presenceCacheProvider.overrideWith(() => cache),
      ]),
    );
    await tester.pumpAndSettle();
    final avatar = find.byWidgetPredicate(
      (widget) => widget is AvatarImage && widget.radius == 9,
    );
    final dots = find.byWidgetPredicate(
      (widget) =>
          widget is Positioned && widget.bottom == -1 && widget.right == -1,
    );
    expect(dots, findsNothing);
    final bounds = tester.getRect(avatar);
    for (final status in ['online', 'away', 'offline']) {
      cache.state = {'alice': status};
      await tester.pumpAndSettle();
      expect(dots, findsOneWidget);
      final dot = tester.widget<Container>(
        find.descendant(of: dots, matching: find.byType(Container)),
      );
      final theme = AppTheme.light();
      expect((dot.decoration as BoxDecoration).color, switch (status) {
        'online' => theme.extension<AppColors>()!.success,
        'away' => theme.extension<AppColors>()!.warning,
        _ => theme.colorScheme.outline,
      });
      expect(tester.getRect(avatar), bounds);
    }
    cache.state = {};
    await tester.pumpAndSettle();
    expect(dots, findsNothing);
  });
}

class _ListPresenceFixture extends PresenceCacheNotifier {
  @override
  Map<String, String> build() => {};
  @override
  void track(List<String> pubkeys) {}
}
