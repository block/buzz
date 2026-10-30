part of '../channels_page_test.dart';

void _homeCommunityFlightTests(
  Widget Function({
    required List<Override> overrides,
    bool realHome,
    bool disableAnimations,
  })
  buildTestable,
) {
  for (final reduced in [false, true]) {
    testWidgets(
      'Home community flight lands at the moved menu (reduced: $reduced)',
      (tester) async {
        debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
        addTearDown(() => debugDefaultTargetPlatformOverride = null);
        tester.view.physicalSize = const Size(390, 844);
        tester.view.devicePixelRatio = 1;
        addTearDown(tester.view.resetPhysicalSize);
        addTearDown(tester.view.resetDevicePixelRatio);
        final communities = [
          for (final id in ['alpha', 'bravo'])
            Community(
              id: id,
              name: id,
              relayUrl: 'wss://$id.example.com',
              addedAt: DateTime(2025),
            ),
        ];
        final notifier = _FakeCommunityListNotifier(communities);
        final loaded = Completer<List<Channel>>();
        SharedPreferences.setMockInitialValues({});
        final prefs = await SharedPreferences.getInstance();
        await tester.pumpWidget(
          buildTestable(
            realHome: true,
            disableAnimations: reduced,
            overrides: [
              savedPrefsProvider.overrideWithValue(prefs),
              channelsProvider.overrideWith(
                () => _FakeNotifier(
                  [],
                  load: () => notifier.activeId == 'bravo'
                      ? loaded.future
                      : Future.value([]),
                ),
              ),
              communityListProvider.overrideWith(() => notifier),
              activeCommunityProvider.overrideWith((ref) async {
                await ref.watch(communityListProvider.future);
                return communities.firstWhere((c) => c.id == notifier.activeId);
              }),
            ],
          ),
        );
        await tester.pumpAndSettle();
        final menu = find.byWidgetPredicate(
          (w) =>
              w is IosMessageCreateButton &&
              w.accessibilityLabel == 'Community menu',
        );
        final destination = tester.getCenter(menu);
        tester.widget<IosMessageCreateButton>(menu).actions.first.onPressed!();
        await tester.pumpAndSettle();
        await tester.tap(find.byKey(const Key('community-switcher-row-bravo')));
        await tester.pump();
        final avatar = find.byKey(const Key('community-flying-avatar'));
        final origin = tester.getCenter(avatar);
        await tester.pump(const Duration(milliseconds: 105));
        if (!reduced) {
          expect(tester.getCenter(avatar), isNot(origin));
          expect(tester.getCenter(avatar), isNot(const Offset(195, 422)));
        }
        await tester.pump(const Duration(milliseconds: 420));
        await tester.pump();
        expect(tester.getCenter(avatar), const Offset(195, 422));
        expect(
          find.byKey(const Key('community-switch-loading')),
          findsOneWidget,
        );
        loaded.complete([]);
        // Advance until readiness starts departure, without skipping the flight.
        for (var i = 0; i < 90; i++) {
          await tester.pump(const Duration(milliseconds: 16));
          if (find
              .byKey(const Key('community-switch-loading'))
              .evaluate()
              .isEmpty) {
            break;
          }
        }
        if (!reduced) {
          await tester.pump(const Duration(milliseconds: 260));
          expect(tester.getCenter(avatar).dy, lessThan(422));
          expect(tester.getCenter(avatar).dy, greaterThan(destination.dy));
          await tester.pump(const Duration(milliseconds: 260));
          expect(
            (tester.getCenter(avatar) - destination).distance,
            lessThan(1),
          );
        }
        await tester.pumpAndSettle();
        expect(avatar, findsNothing);
        expect(notifier.switchedIds, ['bravo']);
        expect(tester.takeException(), isNull);
        debugDefaultTargetPlatformOverride = null;
      },
    );
  }
}
