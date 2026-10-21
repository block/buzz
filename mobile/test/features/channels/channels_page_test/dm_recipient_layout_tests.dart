part of '../channels_page_test.dart';

void dmRecipientLayoutTests(Widget Function(List<Override>) harness) {
  testWidgets(
    'recipient directory lays out only visible rows and reaches end',
    (tester) async {
      tester.view.physicalSize = const Size(390, 800);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.resetPhysicalSize);
      addTearDown(tester.view.resetDevicePixelRatio);
      final users = [
        for (var i = 0; i < 200; i++)
          DirectoryUser(pubkey: 'person-$i', displayName: 'Person $i'),
      ];
      await tester.pumpWidget(
        harness([
          relayDirectoryUsersProvider.overrideWith((ref) async => users),
          relayDirectorySearchProvider.overrideWith(
            (ref, query) async => users,
          ),
        ]),
      );
      await tester.pumpAndSettle();
      await tester.tap(find.byTooltip('Create or start conversation'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('New direct message'));
      await tester.pumpAndSettle();

      if (Platform.environment['COMMUNITY_SCREENSHOTS'] case final directory?) {
        final boundary = tester
            .element(find.byType(BottomSheet).last)
            .findAncestorRenderObjectOfType<RenderRepaintBoundary>()!;
        await tester.runAsync(() async {
          final image = await boundary.toImage(pixelRatio: 2);
          final bytes = await image.toByteData(format: ui.ImageByteFormat.png);
          await Directory(directory).create(recursive: true);
          await File(
            '$directory/dm-directory.png',
          ).writeAsBytes(bytes!.buffer.asUint8List());
          image.dispose();
        });
      }

      final rows = find.byWidgetPredicate(
        (widget) =>
            widget is ListTile &&
            widget.key is ValueKey<String> &&
            (widget.key! as ValueKey<String>).value.startsWith(
              'new-dm-person-',
            ),
      );
      void expectBoundedRows() {
        expect(rows.evaluate().length, inExclusiveRange(0, 30));
        expect(find.byKey(const Key('new-dm-person-person-199')), findsNothing);
      }

      expectBoundedRows();
      final search = find.byKey(const Key('new-dm-search'));
      await tester.enterText(search, 'Person');
      await tester.pump(const Duration(milliseconds: 300));
      await tester.pumpAndSettle();
      expectBoundedRows();
      await tester.tap(find.byKey(const Key('new-dm-person-person-0')));
      await tester.pumpAndSettle();
      expect(find.bySemanticsLabel('Remove Person 0'), findsOneWidget);
      expectBoundedRows();
      await tester.tap(find.bySemanticsLabel('Remove Person 0'));
      await tester.pumpAndSettle();
      expectBoundedRows();

      await tester.enterText(search, 'no such recipient');
      await tester.pump(const Duration(milliseconds: 300));
      await tester.pumpAndSettle();
      expect(find.text('No matching users.'), findsOneWidget);
      expect(rows, findsNothing);
      await tester.enterText(search, '');
      await tester.pump(const Duration(milliseconds: 300));
      await tester.pumpAndSettle();
      expectBoundedRows();

      final scrollable = find
          .descendant(
            of: find.byType(BottomSheet).last,
            matching: find.byWidgetPredicate(
              (widget) =>
                  widget is Scrollable &&
                  widget.axisDirection == AxisDirection.down,
            ),
          )
          .last;
      await tester.scrollUntilVisible(
        find.byKey(const Key('new-dm-person-person-199')),
        300,
        scrollable: scrollable,
        maxScrolls: 100,
      );
      await tester.pumpAndSettle();
      expect(
        find.byKey(const Key('new-dm-person-person-199')).hitTestable(),
        findsOneWidget,
      );
      expect(rows.evaluate().length, lessThan(30));
      expect(tester.takeException(), isNull);

      await tester.tap(find.byTooltip('Close sheet'));
      await tester.pumpAndSettle();
      expect(find.text('New message'), findsNothing);
    },
  );
}
