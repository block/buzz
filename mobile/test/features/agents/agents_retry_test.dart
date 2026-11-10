import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import '../../helpers/remote_agents_fixture.dart';
import 'agents_harness.dart';

void main() {
  testWidgets('Retry repeats a failed ownership lookup', (tester) async {
    final h = await AgentsHarness.create();
    h.registry.listed.add({
      'agent_pubkey': fakeAgent,
      'agent_id': 'agent',
      'agent_name': 'Scout',
      'status': 'BEEKEEPER_AGENT_STATUS_ACTIVE',
    });
    h.relayDown = true;
    await h.open(tester, directory: []);
    expect(find.text('Could not load agents'), findsOneWidget);
    expect(find.byKey(ValueKey(fakeAgent)), findsNothing);

    h.relayDown = false;
    await tester.tap(find.text('Retry'));
    await tester.pumpAndSettle();
    expect(find.text('Could not load agents'), findsNothing);
    expect(find.byKey(ValueKey(fakeAgent)), findsOneWidget);
  });
}
