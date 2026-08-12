import 'package:buzz/features/channels/jump_to_latest_button.dart';
import 'package:buzz/features/channels/unread_divider.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import '../../helpers/widget_helpers.dart';

Future<void> _pump(WidgetTester tester, Widget child) {
  return tester.pumpWidget(WidgetHelpers.testable(child: child));
}

void main() {
  group('JumpToLatestButton', () {
    testWidgets('renders the Latest label', (tester) async {
      await _pump(tester, JumpToLatestButton(onPressed: () {}));

      expect(find.text('Latest'), findsOneWidget);
    });

    testWidgets('invokes onPressed when tapped', (tester) async {
      var taps = 0;
      await _pump(tester, JumpToLatestButton(onPressed: () => taps++));

      await tester.tap(find.text('Latest'));
      await tester.pump();

      expect(taps, 1);
    });

    testWidgets('exposes a surface key so surfaces stay targetable', (
      tester,
    ) async {
      const surfaceKey = ValueKey('thread-jump-to-latest-surface');
      final button = JumpToLatestButton(
        surfaceKey: surfaceKey,
        onPressed: () {},
      );
      await _pump(tester, button);

      expect(find.byKey(surfaceKey), findsOneWidget);
    });

    testWidgets('is marked as a button for assistive tech', (tester) async {
      await _pump(tester, JumpToLatestButton(onPressed: () {}));

      final finder = find.descendant(
        of: find.byType(JumpToLatestButton),
        matching: find.byType(Semantics),
      );
      final semantics = tester.widget<Semantics>(finder.first);

      expect(semantics.properties.button, isTrue);
    });
  });

  group('UnreadDivider', () {
    testWidgets('renders the New label', (tester) async {
      await _pump(tester, const UnreadDivider());

      expect(find.text('New'), findsOneWidget);
    });
  });
}
