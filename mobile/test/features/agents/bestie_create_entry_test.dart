import 'package:buzz/features/agents/create_agent_page.dart';
import 'package:buzz/features/agents/agents_page.dart';
import 'package:buzz/shared/hosted/hosted_environment.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'agents_harness.dart';
import '../../helpers/remote_agents_fixture.dart';
import 'package:shared_preferences/shared_preferences.dart';
import 'dart:convert';

void main() {
  testWidgets(
    'Bestie entry opens creation once and cancellation stays closed',
    (tester) async {
      final h = await AgentsHarness.create();
      await h.open(tester, directory: [], createOnOpen: true);
      expect(find.byType(CreateAgentPage), findsOneWidget);
      expect(find.byType(AgentsPage, skipOffstage: false), findsOneWidget);
      await tester.tap(find.byTooltip('Cancel'));
      await tester.pumpAndSettle();
      expect(find.byType(CreateAgentPage), findsNothing);
      expect(find.byType(AgentsPage), findsOneWidget);
      await tester.tap(find.byTooltip('Close agents'));
      await tester.pumpAndSettle();
      expect(find.text('Open Agents'), findsOneWidget);
      await h.open(
        tester,
        directory: [existingDesktopKeys.public],
        createOnOpen: true,
      );
      expect(find.byType(CreateAgentPage), findsNothing);
      expect(find.byKey(ValueKey(existingDesktopKeys.public)), findsOneWidget);
      await tester.tap(find.byTooltip('Close agents'));
      await tester.pumpAndSettle();
      final prefs = await SharedPreferences.getInstance();
      await prefs.setString(
        'remote-agents:${HostedEnvironment.origin.host}:$fakeSubject:pending',
        jsonEncode({
          'name': 'Saved',
          'request': 'saved-request',
          'community': h.community.relayUrl,
          'owner': fakeOwner.public,
        }),
      );
      h.registry.signedIn = false;
      h.registry.signedIn = true;
      await h.open(tester, directory: [], createOnOpen: true);
      expect(find.byType(CreateAgentPage), findsNothing);
      expect(h.controller.pending?['name'], 'Saved');
      await tester.tap(find.byTooltip('Add agent'));
      await tester.pumpAndSettle();
      expect(find.byTooltip('Resume'), findsOneWidget);
      await tester.pumpWidget(const SizedBox());
    },
    skip: !HostedEnvironment.enabled,
  );
}
