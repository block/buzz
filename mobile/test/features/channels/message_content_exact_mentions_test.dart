import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:buzz/features/channels/message_content.dart';

import '../../helpers/widget_helpers.dart';

void main() {
  testWidgets('authored mention casing survives; bindings still resolve', (
    tester,
  ) async {
    String? tapped;
    await tester.pumpWidget(
      WidgetHelpers.testable(
        child: MessageContent(
          content: '@Helper and @ALICE',
          mentionNames: {
            'a' * 64: 'Helper',
            'b' * 64: 'Helper',
            'c' * 64: 'Alice',
          },
          onMentionTap: (key) => tapped = key,
        ),
      ),
    );
    await tester.pumpAndSettle();
    // Ambiguous plain namesake text keeps its authored casing.
    expect(find.textContaining('@Helper', findRichText: true), findsOneWidget);
    expect(find.textContaining('@helper', findRichText: true), findsNothing);
    // A bound mention matches case-insensitively and still resolves.
    await tester.tap(find.text('Alice'));
    expect(tapped, 'c' * 64);
  });

  for (final tagged in [false, true]) {
    testWidgets(
      'qualified chips preserve authority and narrow layout: $tagged',
      (tester) async {
        tester.view.physicalSize = const Size(320, 640);
        tester.view.devicePixelRatio = 1;
        addTearDown(tester.view.reset);
        final semantics = tester.ensureSemantics();
        final first = 'a' * 64, second = 'b' * 64;
        String? tapped;
        await tester.pumpWidget(
          WidgetHelpers.testable(
            child: MediaQuery(
              data: const MediaQueryData(textScaler: TextScaler.linear(2)),
              child: MessageContent(
                content: '@Scout @Scout ($second)',
                mentionNames: {second: 'Scout', first: 'Scout'},
                tags: [
                  for (final key in [first, if (tagged) second]) ['p', key],
                ],
                onMentionTap: (key) => tapped = key,
              ),
            ),
          ),
        );
        await tester.pumpAndSettle();
        expect(find.text('Scout'), findsNothing);
        final qualified = find.text('Scout (bbbbbbbb…bbbb)');
        if (tagged) {
          await tester.tap(qualified);
          expect(tapped, second);
          expect(
            find.bySemanticsLabel(RegExp('Scout.*$second')),
            findsOneWidget,
          );
        } else {
          expect(qualified, findsNothing);
          expect(tapped, isNull);
        }
        expect(tester.takeException(), isNull);
        semantics.dispose();
      },
    );
  }
}
