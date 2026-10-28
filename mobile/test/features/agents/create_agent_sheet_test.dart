import 'dart:async';

import 'package:buzz/features/agents/agents_sheet.dart';
import 'package:buzz/features/agents/create_agent_sheet.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:nostr/nostr.dart' as nostr;

import 'agents_harness.dart';

void main() {
  final existing = 'a' * 64;

  testWidgets(
    'creation overlays the retained list, then failure and resume both land',
    (tester) async {
      final h = await AgentsHarness.create();
      final controller = h.controller;
      final added = nostr.Keys('0' * 63 + '2').public;
      await h.open(tester, directory: [existing]);
      final list = tester.element(find.byType(AgentsSheet));
      await tester.tap(find.byTooltip('Add agent'));
      await tester.pumpAndSettle();
      expect(find.byType(BottomSheet), findsNWidgets(2));
      final createContext = tester.element(find.byType(CreateAgentSheet));
      expect(Navigator.of(list), isNot(Navigator.of(createContext)));

      controller.gate = Completer<void>();
      await tester.enterText(
        find.descendant(
          of: find.byWidgetPredicate(
            (widget) =>
                widget is Semantics && widget.properties.label == 'Name',
          ),
          matching: find.byType(TextField),
        ),
        'Scout',
      );
      await tester.enterText(
        find.descendant(
          of: find.byWidgetPredicate(
            (widget) =>
                widget is Semantics &&
                widget.properties.label == 'Instructions',
          ),
          matching: find.byType(TextField),
        ),
        'Reply concisely.',
      );
      await tester.pump();
      await tester.tap(find.byTooltip('Create'));
      await tester.pump(const Duration(milliseconds: 100));
      expect(controller.busy, isTrue);
      expect(
        find.descendant(
          of: find.byType(CreateAgentSheet),
          matching: find.byType(LinearProgressIndicator),
        ),
        findsOneWidget,
      );
      expect(
        tester
            .widget<IconButton>(find.widgetWithIcon(IconButton, Icons.check))
            .onPressed,
        isNull,
      );
      controller.gate!.complete();
      await tester.pumpAndSettle();
      expect(find.byType(BottomSheet), findsNWidgets(2));
      expect(find.text('Creation failed. Retry.'), findsOneWidget);
      expect(controller.submittedName, 'Scout');
      expect(controller.submittedInstructions, 'Reply concisely.');

      // The real registry flow then succeeds and returns to the same list.
      controller.gate = null;
      await tester.tap(find.byTooltip('Create'));
      await tester.pumpAndSettle();
      expect(find.byType(BottomSheet), findsOneWidget);
      expect(tester.element(find.byType(AgentsSheet)), same(list));
      expect(find.byKey(ValueKey(added)), findsOneWidget);
      expect(h.registry.calls, contains('publish-policy'));
    },
  );
}
