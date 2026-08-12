import 'package:buzz/features/channels/jump_to_latest_button.dart';
import 'package:buzz/features/channels/unread_divider.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import '../../helpers/widget_helpers.dart';

void main() {
  group('JumpToLatestButton', () {
    testWidgets('renders the Latest label', (tester) async {
      await tester.pumpWidget(
        WidgetHelpers.testable(
          child: JumpToLatestButton(onPressed: () {}),
        ),
      );

      expect(find.text('Latest'), findsOneWidget);
    });

    testWidgets('invokes onPressed when tapped', (tester) async {
      var taps = 0;
      await tester.pumpWidget(
        WidgetHelpers.testable(
          child: JumpToLatestButton(onPressed: () => taps++),
        ),
      );

      await tester.tap(find.text('Latest'));
      await tester.pump();

      expect(taps, 1);
    });

    testWidgets('exposes the surface key so each surface is targetable', (
      tester,
    ) async {
      await tester.pumpWidget(
        WidgetHelpers.testable(
          child: JumpToLatestButton(
            surfaceKey: const ValueKey('thread-jump-to-latest-surface'),
            onPressed: () {},
          ),
        ),
      );

      expect(
        find.byKey(const ValueKey('thread-jump-to-latest-surface')),
        findsOneWidget,
      );
    });

    testWidgets('is marked as a button for assistive tech', (tester) async {
      await tester.pumpWidget(
        WidgetHelpers.testable(
          child: JumpToLatestButton(onPressed: () {}),
        ),
      );

      final semantics = tester.widget<Semantics>(
        find
            .descendant(
              of: find.byType(JumpToLatestButton),
              matching: find.byType(Semantics),
            )
            .first,
      );
      expect(semantics.properties.button, isTrue);
    });
  });

  group('UnreadDivider', () {
    testWidgets('renders the New label', (tester) async {
      await tester.pumpWidget(
        WidgetHelpers.testable(child: const UnreadDivider()),
      );

      expect(find.text('New'), findsOneWidget);
    });
  });
}
