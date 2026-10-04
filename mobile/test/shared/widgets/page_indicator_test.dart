import 'package:buzz/shared/widgets/page_indicator.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import '../../helpers/widget_helpers.dart';

void main() {
  for (final direction in TextDirection.values) {
    for (final window in [
      (count: 3, selected: 1, first: 0, last: 2),
      (count: 20, selected: 0, first: 0, last: 6),
      (count: 20, selected: 10, first: 7, last: 13),
      (count: 20, selected: 19, first: 13, last: 19),
    ]) {
      testWidgets(
        'visible dot centers select their pages in $direction $window',
        (tester) async {
          final selections = <int>[];
          await tester.pumpWidget(
            WidgetHelpers.testable(
              child: Directionality(
                textDirection: direction,
                child: Center(
                  child: SizedBox(
                    width: 390,
                    child: PageIndicator(
                      semanticLabel: 'Photo',
                      count: window.count,
                      selected: window.selected,
                      animateChanges: false,
                      onSelected: selections.add,
                    ),
                  ),
                ),
              ),
            ),
          );
          for (var page = window.first; page <= window.last; page++) {
            final dot = find.byKey(ValueKey('page-indicator-dot-$page'));
            await tester.tapAt(tester.getCenter(dot));
            await tester.pump();
            expect(selections.last, page, reason: 'Tapped visible page $page');
          }
          final first = tester.getCenter(
            find.byKey(ValueKey('page-indicator-dot-${window.first}')),
          );
          final last = tester.getCenter(
            find.byKey(ValueKey('page-indicator-dot-${window.last}')),
          );
          await tester.dragFrom(first, last - first);
          await tester.pump();
          expect(selections.last, window.last);
        },
      );
    }
  }
}
