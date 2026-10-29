import 'package:buzz/features/agents/create_agent_sheet.dart';
import 'package:buzz/shared/hosted/hosted_environment.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'agents_harness.dart';

void main() {
  testWidgets(
    'signing out of Builderlab closes Create agent but keeps Agents open',
    (tester) async {
      final h = await AgentsHarness.create();
      await h.open(tester, directory: ['a' * 64]);
      await tester.tap(find.byTooltip('Add agent'));
      await tester.pumpAndSettle();
      expect(find.byType(CreateAgentSheet), findsOneWidget);
      expect(find.byType(BottomSheet), findsNWidgets(2));

      h.registry.signedIn = false;
      await tester.pumpAndSettle();
      // Only the remote-agent flow closes; community agents stay listed.
      expect(find.byType(CreateAgentSheet), findsNothing);
      expect(find.byType(BottomSheet), findsOneWidget);
      expect(find.byKey(ValueKey('a' * 64)), findsOneWidget);
    },
    skip: !HostedEnvironment.enabled,
  );
}
