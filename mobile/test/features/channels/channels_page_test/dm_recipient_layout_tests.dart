part of '../channels_page_test.dart';

void dmRecipientLayoutTests(Widget Function(List<Override>) harness) {
  testWidgets(
    'recipient directory lays out only visible rows and reaches end',
    (tester) async {
      tester.view.physicalSize = const Size(390, 800);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.resetPhysicalSize);
      addTearDown(tester.view.resetDevicePixelRatio);
      // More matches than one chooser shows: the list stops at its cap.
      final people = {
        for (var i = 0; i < 60; i++)
          _dmKey(i): 'Person ${i.toString().padLeft(2, '0')}',
      };
      final last = _dmKey(mentionSuggestionLimit - 1);
      await tester.pumpWidget(
        harness([
          relaySessionProvider.overrideWith(
            () => RecipientDirectorySession(people),
          ),
        ]),
      );
      await tester.pumpAndSettle();
      await tester.tap(find.byTooltip('Create'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('New DM'));
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
        expect(find.byKey(Key('new-dm-person-$last')), findsNothing);
      }

      expect(
        find.text('Type a name to find people or agents.'),
        findsOneWidget,
      );
      expect(rows, findsNothing);
      await _searchDm(tester, 'Person');
      expectBoundedRows();
      await tester.tap(find.byKey(Key('new-dm-person-${_dmKey(0)}')));
      await tester.pumpAndSettle();
      expect(find.bySemanticsLabel('Remove Person 00'), findsOneWidget);
      // A pick clears the query, as an inserted mention ends its query.
      expect(rows, findsNothing);
      await tester.tap(find.bySemanticsLabel('Remove Person 00'));
      await tester.pumpAndSettle();

      await _searchDm(tester, 'no such recipient');
      expect(find.text('No matching people or agents.'), findsOneWidget);
      expect(rows, findsNothing);
      await _searchDm(tester, 'Person');
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
        find.byKey(Key('new-dm-person-$last')),
        300,
        scrollable: scrollable,
        maxScrolls: 100,
      );
      await tester.pumpAndSettle();
      expect(
        find.byKey(Key('new-dm-person-$last')).hitTestable(),
        findsOneWidget,
      );
      expect(
        find.byKey(Key('new-dm-person-${_dmKey(mentionSuggestionLimit)}')),
        findsNothing,
      );
      expect(rows.evaluate().length, lessThan(30));
      expect(tester.takeException(), isNull);

      await tester.tap(find.byTooltip('Close'));
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('new-dm-search')), findsNothing);
    },
  );
}

/// A 64-hex test key for [index].
String _dmKey(int index) => (index + 1).toRadixString(16).padLeft(64, '0');

/// Types [text] into the New DM search and lets its debounced search finish.
Future<void> _searchDm(WidgetTester tester, String text) async {
  await tester.enterText(find.byKey(const Key('new-dm-search')), text);
  // Build with the new query first, so its debounce starts before time runs.
  await tester.pump();
  await tester.pump(const Duration(milliseconds: 300));
  await tester.pumpAndSettle();
}
