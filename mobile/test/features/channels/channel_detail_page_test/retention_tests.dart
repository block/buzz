part of '../channel_detail_page_test.dart';

void channelRetentionTests() {
  testWidgets('departed channel composer and route are collectible', (
    tester,
  ) async {
    final semantics = tester.ensureSemantics();
    try {
      await tester.pumpWidget(
        _buildTestable(
          messages: [
            _textMsg(id: 'message', pubkey: 'alice', content: 'Hello'),
          ],
          home: Builder(
            builder: (context) => Scaffold(
              body: TextButton(
                onPressed: () => Navigator.of(context).push(
                  MaterialPageRoute<void>(
                    builder: (_) => ChannelDetailPage(channel: _testChannel),
                  ),
                ),
                child: const Text('Open channel'),
              ),
            ),
          ),
        ),
      );
      await tester.pumpAndSettle();

      final retired = <WeakReference<Object>>[];
      for (var visit = 0; visit < 3; visit++) {
        retired.addAll(await _visitAndLeaveChannel(tester));
      }
      // Keep the same app, navigator and semantics owner alive during GC.
      await tester.runAsync(() => forceGC(fullGcCycles: 3));
      expect(
        retired.where((reference) => reference.target != null),
        isEmpty,
        reason: 'Departed routes, composers and their portals must be released',
      );
    } finally {
      semantics.dispose();
    }
  });
}

Future<List<WeakReference<Object>>> _visitAndLeaveChannel(
  WidgetTester tester,
) async {
  await tester.tap(find.text('Open channel'));
  await tester.pumpAndSettle();
  expect(find.text('Message #general'), findsOneWidget);
  final composer = find.byType(ComposeBar);
  final portal = find.descendant(
    of: composer,
    matching: find.byType(OverlayPortal),
  );
  expect(portal, findsWidgets);
  final retired = <WeakReference<Object>>[
    for (final state in tester.stateList(portal)) WeakReference(state),
    WeakReference(tester.element(composer)),
    WeakReference(ModalRoute.of(tester.element(composer))!),
  ];
  Navigator.of(tester.element(composer)).pop();
  await tester.pumpAndSettle();
  expect(find.byType(ChannelDetailPage), findsNothing);
  expect(find.text('Open channel'), findsOneWidget);
  return retired;
}
