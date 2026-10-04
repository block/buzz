import 'package:buzz/shared/widgets/page_indicator.dart';
import 'package:flutter/material.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

import '../../helpers/widget_helpers.dart';

void main() {
  testWidgets(
    'iOS receives initial and updated layout direction',
    (tester) async {
      final updates = <Map<Object?, Object?>>[];
      const channel = MethodChannel('buzz/theme_pagination_glass/54321');
      tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(channel, (
        call,
      ) async {
        if (call.method == 'setState')
          updates.add(call.arguments as Map<Object?, Object?>);
        return null;
      });
      addTearDown(
        () => tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
          channel,
          null,
        ),
      );
      Future<void> pump(TextDirection direction) => tester.pumpWidget(
        WidgetHelpers.testable(
          child: Directionality(
            textDirection: direction,
            child: PageIndicator(
              semanticLabel: 'Photo',
              count: 20,
              selected: 10,
              animateChanges: false,
              onSelected: (_) {},
            ),
          ),
        ),
      );
      await pump(TextDirection.rtl);
      final native = tester.widget<UiKitView>(find.byType(UiKitView));
      expect((native.creationParams as Map)['isRTL'], isTrue);
      native.onPlatformViewCreated!(54321);
      await tester.pump();
      expect(updates.last['isRTL'], isTrue);
      await pump(TextDirection.ltr);
      expect(updates.last['isRTL'], isFalse);
    },
    variant: TargetPlatformVariant.only(TargetPlatform.iOS),
  );

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
