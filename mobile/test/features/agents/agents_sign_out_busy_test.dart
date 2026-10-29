import 'dart:async';

import 'package:buzz/features/agents/create_agent_page.dart';
import 'package:buzz/shared/hosted/hosted_environment.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'agents_harness.dart';

void main() {
  testWidgets('signing out while Create agent is busy still closes it', (
    tester,
  ) async {
    final h = await AgentsHarness.create();
    await h.open(tester, directory: [existingDesktopKeys.public]);
    await tester.tap(find.byTooltip('Add agent'));
    await tester.pumpAndSettle();
    expect(find.byType(CreateAgentPage), findsOneWidget);

    // A request in flight makes Create refuse ordinary pops.
    final pending = Completer<void>();
    unawaited(h.controller.run(() => pending.future));
    await tester.pump();
    expect(h.controller.busy, isTrue);

    h.registry.signedIn = false;
    await tester.pumpAndSettle();
    // Only the remote-agent flow closes; owned desktop agents stay listed.
    expect(find.byType(CreateAgentPage), findsNothing);
    expect(find.byType(BottomSheet), findsNothing);
    expect(find.byKey(ValueKey(existingDesktopKeys.public)), findsOneWidget);
    pending.complete();
    await tester.pumpAndSettle();
  }, skip: !HostedEnvironment.enabled);
}
