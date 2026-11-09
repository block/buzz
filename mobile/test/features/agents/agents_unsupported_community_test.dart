import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:buzz/features/agents/agents_page.dart';
import 'package:buzz/features/agents/create_agent_page.dart';
import 'package:buzz/shared/agents/remote_agents.dart';
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
      final card = find.byKey(ValueKey(existingDesktopKeys.public));
      expect(card, findsOneWidget);
      expect(
        find.descendant(of: card, matching: find.text('On another device')),
        findsOneWidget,
      );
    },
  );

  testWidgets('a failed listing never claims an agent is local', (
    tester,
  ) async {
    final h = await AgentsHarness.create();
    if (!supportsRemoteAgents(h.community)) return;
    const connectivity = MethodChannel(
      'dev.fluttercommunity.plus/connectivity_status',
    );
    final messenger = tester.binding.defaultBinaryMessenger;
    messenger.setMockMethodCallHandler(connectivity, (_) async => null);
    addTearDown(() => messenger.setMockMethodCallHandler(connectivity, null));
    h.registry.failListing = true;
    await h.open(tester, directory: [existingDesktopKeys.public]);
    expect(h.controller.listingFailed, isTrue);
    final card = find.byKey(ValueKey(existingDesktopKeys.public));
    final local = find.descendant(
      of: card,
      matching: find.text('On another device'),
    );
    expect(local, findsNothing);

    await tester.tap(card);
    await tester.pumpAndSettle();
    expect(
      find.textContaining('Couldn’t check whether this agent runs on'),
      findsOneWidget,
    );

    // A later listing confirms the location while the profile is open.
    h.registry.failListing = false;
    await h.controller.run(h.controller.refresh);
    await tester.pumpAndSettle();
    expect(find.textContaining('Couldn’t check'), findsNothing);
    expect(
      find.textContaining('Runs on another of your devices'),
      findsOneWidget,
    );
    await tester.binding.handlePopRoute();
    await tester.pumpAndSettle();
    expect(local, findsOneWidget);

    // A failed refresh after that success leaves a stale snapshot, which
    // can't rule out a remote agent created since.
    h.registry.failListing = true;
    await h.controller.run(h.controller.refresh);
    await tester.pumpAndSettle();
    expect(h.controller.listingFailed, isTrue);
    expect(local, findsNothing);
  });
}
