import 'package:buzz/features/channels/message_footer_layout.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  testWidgets('footer global transforms and semantics include overlap offset', (
    tester,
  ) async {
    final semantics = tester.ensureSemantics();

    await tester.pumpWidget(
      MaterialApp(
        home: Align(
          alignment: Alignment.topLeft,
          child: MessageFooterLayout(
            overlap: 8,
            footer: Semantics(
              label: 'Reaction footer',
              child: const SizedBox(
                key: ValueKey('footer'),
                width: 100,
                height: 32,
              ),
            ),
            child: const SizedBox(
              key: ValueKey('content'),
              width: 200,
              height: 60,
            ),
          ),
        ),
      ),
    );
    expect(
      tester.getTopLeft(find.byKey(const ValueKey('footer'))),
      tester.getTopLeft(find.byKey(const ValueKey('content'))) +
          const Offset(0, 52),
    );
    final node = tester.getSemantics(find.bySemanticsLabel('Reaction footer'));
    expect(node.transform?.getTranslation().y, 52);
    semantics.dispose();
  });

  for (final reduced in [false, true]) {
    testWidgets('reaction spacing animates both ways, reduced=$reduced', (
      tester,
    ) async {
      Widget view(bool reaction) => MaterialApp(
        home: MediaQuery(
          data: MediaQueryData(disableAnimations: reduced),
          child: Align(
            alignment: Alignment.topLeft,
            child: MessageFooterLayout(
              animateSize: true,
              overlap: reaction ? 8 : 0,
              footer: SizedBox(width: 100, height: reaction ? 32 : 0),
              child: const SizedBox(width: 200, height: 60),
            ),
          ),
        ),
      );
      double height() =>
          tester.getSize(find.byType(MessageFooterLayout)).height;
      await tester.pumpWidget(view(false));
      await tester.pumpWidget(view(true));
      await tester.pump(const Duration(milliseconds: 100));
      expect(
        height(),
        reduced ? equals(84) : allOf(greaterThan(60), lessThan(84)),
      );
      await tester.pumpAndSettle();
      expect(height(), 84);
      await tester.pumpWidget(view(false));
      await tester.pump(const Duration(milliseconds: 100));
      expect(
        height(),
        reduced ? equals(60) : allOf(greaterThan(60), lessThan(84)),
      );
      await tester.pumpAndSettle();
      expect(height(), 60);
    });
  }

  testWidgets(
    'overlapping reaction area is tappable and reserves its protrusion',
    (tester) async {
      var taps = 0;
      await tester.pumpWidget(
        MaterialApp(
          home: Align(
            alignment: Alignment.topLeft,
            child: MessageFooterLayout(
              overlap: 8,
              footer: GestureDetector(
                onTap: () => taps++,
                behavior: HitTestBehavior.opaque,
                child: const SizedBox(
                  key: ValueKey('reactions'),
                  width: 100,
                  height: 32,
                ),
              ),
              child: const SizedBox(
                key: ValueKey('body'),
                width: 200,
                height: 60,
              ),
            ),
          ),
        ),
      );
      final body = tester.getRect(find.byKey(const ValueKey('body')));
      final badge = tester.getRect(find.byKey(const ValueKey('reactions')));
      expect(badge.top, body.bottom - 8);
      expect(tester.getSize(find.byType(MessageFooterLayout)).height, 84);
      await tester.tapAt(Offset(badge.left + 10, badge.top + 2));
      expect(taps, 1);
    },
  );
}
