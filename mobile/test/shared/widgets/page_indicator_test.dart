import 'package:buzz/shared/widgets/page_indicator.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter/semantics.dart';
import 'package:flutter/foundation.dart';
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
        if (call.method == 'setState') {
          updates.add(call.arguments as Map<Object?, Object?>);
        }
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

  testWidgets(
    'pagination has a single platform semantics owner',
    (tester) async {
      final semantics = tester.ensureSemantics();
      try {
        final selected = ValueNotifier(1);
        addTearDown(selected.dispose);
        await tester.pumpWidget(
          WidgetHelpers.testable(
            child: ValueListenableBuilder<int>(
              valueListenable: selected,
              builder: (context, page, _) => PageIndicator(
                semanticLabel: 'Photo',
                count: 3,
                selected: page,
                animateChanges: false,
                onSelected: (page) => selected.value = page,
              ),
            ),
          ),
        );
        List<SemanticsNode> adjustableNodes() {
          final nodes = <SemanticsNode>[];
          void visit(SemanticsNode node) {
            final data = node.getSemanticsData();
            if (data.hasAction(SemanticsAction.increase) ||
                data.hasAction(SemanticsAction.decrease)) {
              nodes.add(node);
            }
            node.visitChildren((child) {
              visit(child);
              return true;
            });
          }

          visit(
            tester
                .renderObject(find.byType(PageIndicator))
                .owner!
                .semanticsOwner!
                .rootSemanticsNode!,
          );
          return nodes;
        }

        if (defaultTargetPlatform == TargetPlatform.iOS) {
          // Flutter must preserve the native view bridge without adding a second
          // adjustable owner. The UIKit regression checks that view's actions.
          expect(adjustableNodes(), isEmpty);
          final native = tester.widget<UiKitView>(find.byType(UiKitView));
          expect((native.creationParams as Map)['accessibilityLabel'], 'Photo');
          expect((native.creationParams as Map)['selected'], 1);
          expect((native.creationParams as Map)['count'], 3);
          expect(
            find.ancestor(
              of: find.byType(UiKitView),
              matching: find.byType(ExcludeSemantics),
            ),
            findsNothing,
          );
        } else {
          expect(adjustableNodes(), hasLength(1));
          var node = adjustableNodes().single;
          expect(node.label, 'Photo 2 of 3');
          expect(node.value, '2');
          tester
              .renderObject(find.byType(PageIndicator))
              .owner!
              .semanticsOwner!
              .performAction(node.id, SemanticsAction.increase);
          await tester.pump();
          expect(selected.value, 2);
          node = adjustableNodes().single;
          expect(node.label, 'Photo 3 of 3');
          expect(node.value, '3');
          expect(
            node.getSemanticsData().hasAction(SemanticsAction.increase),
            isFalse,
          );
          tester
              .renderObject(find.byType(PageIndicator))
              .owner!
              .semanticsOwner!
              .performAction(node.id, SemanticsAction.decrease);
          await tester.pump();
          expect(selected.value, 1);
          expect(adjustableNodes(), hasLength(1));
        }
      } finally {
        semantics.dispose();
      }
    },
    variant: TargetPlatformVariant({
      TargetPlatform.iOS,
      TargetPlatform.android,
    }),
  );

  for (final direction in TextDirection.values) {
    for (final cancel in [false, true]) {
      testWidgets(
        'scrub window stays fixed across rebuilds in $direction, cancel=$cancel',
        (tester) async {
          final selected = ValueNotifier(10);
          addTearDown(selected.dispose);
          await tester.pumpWidget(
            WidgetHelpers.testable(
              child: Directionality(
                textDirection: direction,
                child: Center(
                  child: SizedBox(
                    width: 390,
                    child: ValueListenableBuilder<int>(
                      valueListenable: selected,
                      builder: (context, page, _) => PageIndicator(
                        semanticLabel: 'Photo',
                        count: 20,
                        selected: page,
                        animateChanges: false,
                        onSelected: (page) => selected.value = page,
                      ),
                    ),
                  ),
                ),
              ),
            ),
          );
          Offset center(int page) => tester.getCenter(
            find.byKey(ValueKey('page-indicator-dot-$page')),
          );
          final target = center(12);
          final gesture = await tester.startGesture(center(10));
          await gesture.moveTo(target);
          await tester.pump();
          expect(selected.value, 12);
          for (var event = 0; event < 6; event++) {
            // Vertical jitter emits new pointer events at an unchanged scrub x.
            await gesture.moveTo(target + Offset(0, event.isEven ? 1 : 0));
            await tester.pump();
            expect(selected.value, 12, reason: 'Held pointer must not cascade');
            expect(
              center(12).dx,
              target.dx,
              reason: 'Rendered window stays under the pointer',
            );
          }
          if (cancel) {
            await gesture.cancel();
          } else {
            await gesture.up();
          }
          await tester.pump();
          expect(selected.value, 12);
          expect(
            center(12).dx,
            isNot(target.dx),
            reason: 'Release recenters the window',
          );
          // The next gesture must snapshot the new window after end OR cancel.
          final next = await tester.startGesture(center(12));
          await next.moveTo(center(14));
          await tester.pump();
          expect(selected.value, 14);
          await next.up();
          await tester.pump();
          await tester.tapAt(center(15));
          await tester.pump();
          expect(
            selected.value,
            15,
            reason: 'A tap uses the current window once',
          );
        },
      );
    }
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
