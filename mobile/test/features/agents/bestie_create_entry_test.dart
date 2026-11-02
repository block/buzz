import 'package:buzz/features/agents/create_agent_page.dart';
import 'package:buzz/features/agents/agents_page.dart';
import 'package:buzz/shared/hosted/hosted_environment.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'agents_harness.dart';

void main() {
  testWidgets(
    'Bestie entry opens creation once and cancellation stays closed',
    (tester) async {
      final h = await AgentsHarness.create();
      await h.open(tester, directory: [], createOnOpen: true);
      expect(find.byType(CreateAgentPage), findsOneWidget);
      expect(find.byType(AgentsPage, skipOffstage: false), findsNothing);
      await tester.tap(find.byTooltip('Cancel'));
      await tester.pumpAndSettle();
      expect(find.byType(CreateAgentPage), findsNothing);
      expect(find.text('Open Agents'), findsOneWidget);
      expect(find.byType(AgentsPage, skipOffstage: false), findsNothing);
      await tester.pumpWidget(const SizedBox());
    },
    skip: !HostedEnvironment.enabled,
  );
}
