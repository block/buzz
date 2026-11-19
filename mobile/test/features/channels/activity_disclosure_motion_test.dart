import 'package:buzz/features/channels/activity_disclosure_motion.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  for (final reduced in [false, true]) {
    testWidgets('disclosure grows and reverses, reduced=$reduced', (
      tester,
    ) async {
      var expanded = false;
      late StateSetter update;
      await tester.pumpWidget(
        MaterialApp(
          theme: AppTheme.light(),
          home: MediaQuery(
            data: MediaQueryData(disableAnimations: reduced),
            child: Scaffold(
              body: Align(
                alignment: Alignment.topLeft,
                child: StatefulBuilder(
                  builder: (context, setState) {
                    update = setState;
                    return ActivityDisclosureMotion(
                      expanded: expanded,
                      onToggle: () {},
                      child: SizedBox(
                        width: expanded ? 280 : 48,
                        height: expanded ? 240 : 48,
                      ),
                    );
                  },
                ),
              ),
            ),
          ),
        ),
      );
      Size size() => tester.getSize(find.byType(ActivityDisclosureMotion));
      expect(size(), const Size(48, 48));
      update(() => expanded = true);
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 80));
      if (reduced) {
        expect(size(), const Size(280, 240));
        expect(find.byType(AnimatedSize), findsNothing);
      } else {
        expect(size().height, greaterThan(48));
        expect(size().height, lessThan(240));
        final surface = tester.widget<AnimatedContainer>(
          find.byType(AnimatedContainer),
        );
        expect(surface.clipBehavior, Clip.antiAlias);
      }
      // Reverse mid-flight, rather than letting every transition finish first.
      update(() => expanded = false);
      await tester.pump();
      await tester.pumpAndSettle();
      expect(size(), const Size(48, 48));
      expect(tester.takeException(), isNull);
    });
  }
}
