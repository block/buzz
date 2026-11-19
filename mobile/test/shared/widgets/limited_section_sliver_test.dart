import 'package:buzz/shared/widgets/limited_section_sliver.dart';
import 'package:buzz/features/channels/me/me_section.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  Widget page(int count, {bool reducedMotion = false}) => MaterialApp(
    home: MediaQuery(
      data: MediaQueryData(disableAnimations: reducedMotion),
      child: Scaffold(
        body: CustomScrollView(
          slivers: [
            LimitedSectionSliver(
              itemCount: count,
              itemBuilder: (context, index) =>
                  SizedBox(height: 32, child: Text('Conversation $index')),
            ),
          ],
        ),
      ),
    ),
  );

  testWidgets('seven rows need no disclosure', (tester) async {
    await tester.pumpWidget(page(7));
    expect(find.text('Conversation 6'), findsOneWidget);
    expect(find.text('Show more'), findsNothing);
    expect(find.text('Show fewer'), findsNothing);
  });

  for (final reducedMotion in [false, true]) {
    testWidgets('reveals and collapses overflow (reduced: $reducedMotion)', (
      tester,
    ) async {
      await tester.pumpWidget(page(11, reducedMotion: reducedMotion));
      expect(find.text('Conversation 6'), findsOneWidget);
      expect(find.text('Conversation 7'), findsNothing);
      await tester.tap(find.text('Show more'));
      await tester.pumpAndSettle();
      expect(find.text('Conversation 10'), findsOneWidget);
      expect(find.text('Show fewer'), findsOneWidget);
      await tester.tap(find.text('Show fewer'));
      await tester.pumpAndSettle();
      expect(find.text('Conversation 7'), findsNothing);
      expect(find.text('Show more'), findsOneWidget);
      expect(tester.takeException(), isNull);
    });
  }

  testWidgets('can reverse an in-progress reveal', (tester) async {
    await tester.pumpWidget(page(11));
    await tester.tap(find.text('Show more'));
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 60));
    await tester.tap(find.text('Show fewer'));
    await tester.pumpAndSettle();
    expect(find.text('Conversation 7'), findsNothing);
    expect(tester.takeException(), isNull);
  });
  testWidgets('Solo retains the top-level section collapse', (tester) async {
    await tester.pumpWidget(
      MaterialApp(
        home: Scaffold(
          body: CustomScrollView(
            slivers: [
              MeSection(
                sliver: true,
                label: 'Workspace',
                children: List.generate(
                  10,
                  (index) => SizedBox(height: 32, child: Text('Solo $index')),
                ),
              ),
            ],
          ),
        ),
      ),
    );
    expect(find.text('Solo 7'), findsNothing);
    await tester.tap(find.text('Show more'));
    await tester.pumpAndSettle();
    expect(find.text('Solo 9'), findsOneWidget);
    await tester.tap(find.text('Workspace'));
    await tester.pumpAndSettle();
    expect(find.text('Solo 0'), findsNothing);
    expect(find.text('Show fewer'), findsNothing);
    await tester.tap(find.text('Workspace'));
    await tester.pumpAndSettle();
    expect(find.text('Solo 0'), findsOneWidget);
    expect(tester.takeException(), isNull);
  });
}
