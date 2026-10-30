import 'package:buzz/features/agents/create_agent_page.dart';
import 'package:buzz/shared/hosted/hosted_environment.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'agents_harness.dart';

void main() {
  testWidgets(
    'signing out of Builderlab closes Create agent but keeps Agents open',
    (tester) async {
      final h = await AgentsHarness.create();
      await h.open(tester, directory: [existingDesktopKeys.public]);
      await tester.tap(find.byTooltip('Add agent'));
      await tester.pumpAndSettle();
      expect(find.byType(CreateAgentPage), findsOneWidget);
      await tester.tap(find.text('Choose avatar'));
      await tester.pumpAndSettle();

      h.registry.signedIn = false;
      await tester.pumpAndSettle();
      // Only the remote-agent flow closes; owned desktop agents stay listed.
      expect(find.byType(CreateAgentPage, skipOffstage: false), findsNothing);
      await tester.tap(find.byTooltip('Cancel'));
      await tester.pumpAndSettle();
      expect(find.byType(BottomSheet), findsNothing);
      expect(find.byKey(ValueKey(existingDesktopKeys.public)), findsOneWidget);
    },
    skip: !HostedEnvironment.enabled,
  );
}
