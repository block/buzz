import 'package:buzz/shared/theme/theme.dart';
import 'package:buzz/shared/widgets/buzz_callout.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  for (final (width, scale) in [(320.0, 1.0), (320.0, 3.0), (1024.0, 1.0)]) {
    testWidgets(
      'stays on screen beside an edge target, $width wide at ${scale}x',
      (tester) async {
        tester.view.physicalSize = Size(width, 700);
        tester.view.devicePixelRatio = 1;
        tester.platformDispatcher.textScaleFactorTestValue = scale;
        addTearDown(tester.view.reset);
        addTearDown(tester.platformDispatcher.clearTextScaleFactorTestValue);
        await tester.pumpWidget(
          MaterialApp(
            theme: AppTheme.dark(),
            home: Scaffold(
              body: Align(
                alignment: Alignment.bottomRight,
                child: BuzzCallout(
                  message: 'A tip long enough to need room',
                  visible: true,
                  onDismiss: () {},
                  child: const SizedBox.square(
                    key: Key('target'),
                    dimension: 56,
                  ),
                ),
              ),
            ),
          ),
        );
        await tester.pump();
        final tip = tester.getRect(
          find.ancestor(
            of: find.text('A tip long enough to need room'),
            matching: find.byType(TapRegion),
          ),
        );
        final target = tester.getRect(find.byKey(const Key('target')));
        expect(tip.right, lessThanOrEqualTo(width - Grid.xs));
        expect(tip.left, greaterThanOrEqualTo(Grid.xs));
        expect(tip.bottom, lessThan(target.top));
      },
    );
  }
}
