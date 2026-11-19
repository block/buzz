part of '../channels_page_test.dart';

void _sectionLimitTests(
  Widget Function({required List<Override> overrides}) build,
) {
  for (final type in ['stream', 'dm']) {
    testWidgets('$type section limits the recent roster and reveals all', (
      tester,
    ) async {
      tester.view.physicalSize = const Size(430, 1200);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.resetPhysicalSize);
      addTearDown(tester.view.resetDevicePixelRatio);
      final channels = [
        for (var index = 0; index < 10; index++)
          Channel(
            id: 'item-$index',
            name: 'Item $index',
            channelType: type,
            visibility: 'open',
            description: '',
            createdBy: 'abc',
            createdAt: DateTime(2025),
            lastMessageAt: DateTime(2026, 1, index + 1),
            memberCount: 2,
            isMember: true,
          ),
      ];
      await tester.pumpWidget(
        build(
          overrides: [
            channelsProvider.overrideWith(() => _FakeNotifier(channels)),
            channelSortProvider.overrideWith(
              () => _EditableTestSort(recent: true),
            ),
          ],
        ),
      );
      await tester.pumpAndSettle();
      expect(find.text('Item 9'), findsOneWidget);
      expect(find.text('Item 3'), findsOneWidget);
      expect(find.text('Item 2'), findsNothing);
      expect(
        tester.getTopLeft(find.text('Item 9')).dy,
        lessThan(tester.getTopLeft(find.text('Item 3')).dy),
      );
      final more = find.text('Show more');
      final previous = find.text('Item 3');
      expect(
        tester.getTopLeft(more).dx,
        closeTo(tester.getTopLeft(previous).dx, 0.01),
      );
      expect(
        tester.getTopLeft(more).dy - tester.getTopLeft(previous).dy,
        closeTo(
          tester.getTopLeft(previous).dy -
              tester.getTopLeft(find.text('Item 4')).dy,
          0.01,
        ),
      );
      final moreStyle = tester.widget<Text>(more).style!;
      final rowStyle = tester.widget<Text>(previous).style!;
      expect(moreStyle.fontSize, rowStyle.fontSize);
      expect(moreStyle.height, rowStyle.height);
      expect(moreStyle.fontFamily, rowStyle.fontFamily);
      await tester.tap(more);
      await tester.pumpAndSettle();
      expect(find.text('Item 0'), findsOneWidget);
      await tester.tap(find.text('Show fewer'));
      await tester.pumpAndSettle();
      expect(find.text('Item 2'), findsNothing);
      expect(tester.takeException(), isNull);
    });
  }
}

class _EditableTestSort extends ChannelSortNotifier {
  _EditableTestSort({this.recent = false});
  final bool recent;
  @override
  ChannelSortState build() => ChannelSortState(
    isReady: true,
    store: ChannelSortStore(
      groups: recent
          ? {'channels': ChannelSortMode.recent, 'dms': ChannelSortMode.recent}
          : {},
    ),
  );

  @override
  void setSortModeFor(
    String groupKey,
    ChannelSortMode mode, {
    Iterable<String>? liveSectionIds,
  }) {
    state = ChannelSortState(
      isReady: true,
      store: ChannelSortStore(groups: {...state.store.groups, groupKey: mode}),
      version: state.version + 1,
    );
  }
}
