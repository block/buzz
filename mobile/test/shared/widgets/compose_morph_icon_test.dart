import 'package:buzz/shared/widgets/compose_morph_icon.dart';
import 'package:buzz/shared/theme/buzz_icons.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  testWidgets('morph reverses in place and settles at each Tabler icon', (
    tester,
  ) async {
    Widget app(bool compose, {bool reduced = false}) => MaterialApp(
      home: MediaQuery(
        data: MediaQueryData(disableAnimations: reduced),
        child: Center(
          child: ComposeMorphIcon(
            compose: compose,
            color: Colors.black,
            surfaceBuilder: (icon) => SizedBox(
              key: const ValueKey('button-surface'),
              width: 56,
              height: 56,
              child: Center(child: icon),
            ),
          ),
        ),
      ),
    );
    await tester.pumpWidget(app(false));
    expect(find.byIcon(BuzzIcons.plus), findsOneWidget);
    await tester.pumpWidget(app(true));
    await tester.pump(const Duration(milliseconds: 100));
    expect(find.byIcon(BuzzIcons.edit), findsNothing);
    Transform rotation() => tester.widget<Transform>(
      find.byKey(const ValueKey('compose-icon-rotation')),
    );
    expect(rotation().transform.entry(1, 0), greaterThan(0));
    final scale = tester.widget<Transform>(
      find.byKey(const ValueKey('compose-surface-scale')),
    );
    expect(scale.transform.entry(0, 0), greaterThan(1));
    expect(
      find.ancestor(
        of: find.byKey(const ValueKey('button-surface')),
        matching: find.byKey(const ValueKey('compose-icon-rotation')),
      ),
      findsNothing,
    );
    await tester.pumpWidget(app(false));
    await tester.pumpAndSettle();
    expect(find.byIcon(BuzzIcons.plus), findsOneWidget);
    await tester.pumpWidget(app(true));
    await tester.pumpAndSettle();
    expect(find.byIcon(BuzzIcons.edit), findsOneWidget);
    expect(rotation().transform.entry(1, 0), closeTo(1, 0.001));
    await tester.pump(const Duration(milliseconds: 500));
    expect(rotation().transform.entry(1, 0), closeTo(1, 0.001));
    await tester.pumpWidget(app(false));
    await tester.pump(const Duration(milliseconds: 100));
    expect(rotation().transform.entry(1, 0), inExclusiveRange(0, 1));
    await tester.pumpWidget(app(false, reduced: true));
    await tester.pump();
    expect(find.byIcon(BuzzIcons.plus), findsOneWidget);
    expect(tester.takeException(), isNull);
  });
}
