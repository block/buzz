import 'package:flutter/material.dart';
import 'package:buzz/features/agents/agents_page.dart';
import 'package:buzz/features/agents/create_agent_page.dart';
import 'package:flutter_test/flutter_test.dart';

import 'agents_harness.dart';

void main() {
  testWidgets(
    'a remote listing failure is hidden where remote agents are off',
    (tester) async {
      final h = await AgentsHarness.create(relayUrl: 'wss://relay.example');
      h.registry.failListing = true;
      await h.open(
        tester,
        directory: [existingDesktopKeys.public],
        createOnOpen: true,
      );

      expect(h.controller.listingFailed, isTrue);
      expect(find.text('Could not load agents'), findsNothing);
      expect(find.byType(AgentsPage), findsOneWidget);
      expect(find.byType(CreateAgentPage), findsNothing);
      expect(find.byKey(ValueKey(existingDesktopKeys.public)), findsOneWidget);
    },
  );
}
