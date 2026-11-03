import 'dart:async';
import 'package:buzz/features/agents/create_agent_page.dart';
import 'package:buzz/shared/hosted/hosted_environment.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'agents_harness.dart';
import '../../helpers/remote_agents_fixture.dart';

void main() {
  testWidgets('Bestie waits for restore and displays binding failures', (
    tester,
  ) async {
    final h = await AgentsHarness.create();
    final gate = Completer<void>();
    h.registry.restoreGate = gate;
    h.registry.signedIn = false;
    // Simulate a pre-existing cold-start load that owns the controller lock.
    final loading = h.controller.run(h.controller.refresh);
    await h.open(tester, directory: [], createOnOpen: true, settle: false);
    expect(find.byType(LinearProgressIndicator), findsOneWidget);
    expect(find.byType(CreateAgentPage), findsNothing);
    h.registry.bound = 'f' * 64;
    h.registry.signedIn = true;
    gate.complete();
    await loading;
    await tester.pumpAndSettle();
    expect(find.text('Buzz identity: mismatch.'), findsOneWidget);
    expect(find.byType(CreateAgentPage), findsNothing);
    expect(tester.takeException(), isNull);
    await tester.tap(find.byTooltip('Close agents'));
    await tester.pumpAndSettle();
    h.registry.bound = fakeOwner.public;
    await tester.tap(find.text('Open Agents'));
    await tester.pumpAndSettle();
    expect(find.byType(CreateAgentPage), findsOneWidget);
    await tester.pumpWidget(const SizedBox());
  }, skip: !HostedEnvironment.enabled);
}
