import 'dart:convert';
import 'package:buzz/features/agents/create_agent_page.dart';
import 'package:buzz/shared/hosted/hosted_environment.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shared_preferences/shared_preferences.dart';
import 'agents_harness.dart';
import '../../helpers/remote_agents_fixture.dart';

void main() {
  testWidgets('Bestie keeps unfinished setup accessible in Agents', (
    tester,
  ) async {
    final h = await AgentsHarness.create();
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
    await h.open(tester, directory: [], createOnOpen: true);
    expect(find.byType(CreateAgentPage), findsNothing);
    expect(h.controller.pending?['name'], 'Saved');
    await tester.tap(find.byTooltip('Add agent'));
    await tester.pumpAndSettle();
    expect(find.byTooltip('Resume'), findsOneWidget);
    await tester.pumpWidget(const SizedBox());
  }, skip: !HostedEnvironment.enabled);
}
