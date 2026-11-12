import 'package:buzz/shared/widgets/dm_header_cluster.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  testWidgets(
    'conversation cluster fits one through many portraits without overlap',
    (tester) async {
      for (final count in [1, 2, 3, 4, 8]) {
        await tester.pumpWidget(
          MaterialApp(
            home: Center(
              child: DmHeaderCluster(
                count: count,
                builder: (i, size) => SizedBox.square(
                  key: ValueKey('portrait-$i'),
                  dimension: size,
                ),
              ),
            ),
          ),
        );
        final cluster = tester.getRect(find.byType(DmHeaderCluster));
        expect(cluster.size, const Size(64, 64));
        final visible = count.clamp(0, 4);
        final rectangles = [
          for (var i = 0; i < visible; i++)
            tester.getRect(find.byKey(ValueKey('portrait-$i'))),
        ];
        expect(find.byKey(ValueKey('portrait-$visible')), findsNothing);
        for (var i = 0; i < rectangles.length; i++) {
          final portrait = rectangles[i];
          expect(
            (portrait.center - cluster.center).distance + portrait.width / 2,
            lessThanOrEqualTo(32.01),
          );
          for (var j = i + 1; j < rectangles.length; j++) {
            expect(
              (portrait.center - rectangles[j].center).distance,
              greaterThanOrEqualTo((portrait.width + rectangles[j].width) / 2),
            );
          }
        }
        if (count > 1) {
          expect(rectangles.map((r) => r.width).toSet().length, visible);
        }
      }
    },
  );
}
