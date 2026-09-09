import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:buzz/features/channels/message_content.dart';
import '../../helpers/widget_helpers.dart';

void main() {
  testWidgets('untagged qualifiers cannot become shorter clickable aliases', (
    tester,
  ) async {
    await tester.pumpWidget(
      WidgetHelpers.testable(
        child: MessageContent(
          content: '@Scout (${'b' * 64})',
          mentionNames: {'a' * 64: 'Scout'},
          onMentionTap: (_) => fail('unbound text is not a profile target'),
        ),
      ),
    );
    await tester.pumpAndSettle();
    expect(find.text('Scout'), findsNothing);
    expect(tester.takeException(), isNull);
  });

  testWidgets('qualified chips wrap in narrow layouts at large text sizes', (
    tester,
  ) async {
    tester.view.physicalSize = const Size(320, 640);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.reset);
    await tester.pumpWidget(
      WidgetHelpers.testable(
        child: MediaQuery(
          data: const MediaQueryData(textScaler: TextScaler.linear(2)),
          child: MessageContent(
            content: '@Scout (${'b' * 64})',
            mentionNames: {'b' * 64: 'Scout'},
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();
    expect(tester.takeException(), isNull);
  });

  testWidgets('namesake chips resolve exact tagged keys, not tag order', (
    tester,
  ) async {
    final semantics = tester.ensureSemantics();
    final first = 'a' * 64;
    final second = 'b' * 64;
    String? tapped;
    await tester.pumpWidget(
      WidgetHelpers.testable(
        child: MessageContent(
          content: '@Scout @Scout ($second)',
          mentionNames: {second: 'Scout', first: 'Scout'},
          onMentionTap: (key) => tapped = key,
        ),
      ),
    );
    await tester.pumpAndSettle();
    await tester.tap(find.text('Scout'));
    expect(tapped, first);
    await tester.tap(find.text('Scout (bbbbbbbb…bbbb)'));
    expect(tapped, second);
    expect(find.bySemanticsLabel(RegExp('Scout.*$second')), findsOneWidget);
    expect(tester.takeException(), isNull);
    semantics.dispose();
  });
  testWidgets('historical namesakes preserve targets with ordinary siblings', (
    tester,
  ) async {
    final first = 'a' * 64, second = 'b' * 64, sibling = 'c' * 64;
    for (final firstName in ['Scout', 'Renamed Scout', first]) {
      for (final secondName in ['Scout', 'Renamed Scout', second]) {
        for (final reverse in [false, true]) {
          for (final ambiguous in [false, true]) {
            final entries = {
              first: firstName,
              second: secondName,
              sibling: 'Alice',
              if (ambiguous) 'd' * 64: 'Unknown',
            }.entries.toList();
            String? tapped;
            await tester.pumpWidget(
              WidgetHelpers.testable(
                child: MessageContent(
                  content: '@Scout @Scout ($second) @Alice',
                  mentionNames: Map.fromEntries(
                    reverse ? entries.reversed : entries,
                  ),
                  onMentionTap: (key) => tapped = key,
                ),
              ),
            );
            await tester.pumpAndSettle();
            if (ambiguous) {
              expect(find.text(firstName), findsNothing);
            } else {
              await tester.tap(find.text(firstName));
              expect(tapped, first);
            }
            await tester.tap(find.text('Scout (bbbbbbbb…bbbb)'));
            expect(tapped, second);
            await tester.tap(find.text('Alice'));
            expect(tapped, sibling);
          }
        }
      }
    }
  });
}
