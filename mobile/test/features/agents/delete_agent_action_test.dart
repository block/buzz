import 'dart:async';
import 'package:buzz/shared/me/default_agent.dart';
import 'package:buzz/shared/me/default_agent_writer.dart';
import 'package:buzz/shared/me/me_sync.dart';
import 'package:buzz/shared/relay/relay.dart';
import '../../helpers/remote_agents_fixture.dart';
import 'package:buzz/features/agents/delete_agent_action.dart';
import 'package:buzz/shared/community/community_provider.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;

import 'agents_harness.dart';

void main() {
  testWidgets('Delete appears only for owned remote agents and needs confirm', (
    tester,
  ) async {
    final h = await AgentsHarness.create();
    final controller = h.controller;
    final choices = StreamController<String?>();
    addTearDown(choices.close);
    final sync = MeSync(
      h.relay,
      RelayConfig(baseUrl: 'https://test', nsec: fakeOwner.nsec),
    );
    final agent = nostr.Keys('0' * 63 + '2').public;
    h.registry.listed = [
      {'agent_pubkey': agent, 'agent_id': 'remote-id', 'agent_name': 'Scout'},
    ];
    await controller.run(controller.refresh);
    h.registry.calls.clear();
    await tester.pumpWidget(
      ProviderScope(
        overrides: [
          defaultAgentProvider.overrideWith((ref) => choices.stream),
          meSyncProvider.overrideWithValue(sync),
          relaySessionProvider.overrideWith(() => h.relay),
          activeCommunityProvider.overrideWith((ref) async => h.community),
        ],
        child: MaterialApp(
          theme: AppTheme.light(),
          home: Scaffold(
            body: Column(
              children: [
                DeleteAgentAction(controller: controller, pubkey: agent),
                DeleteAgentAction(controller: controller, pubkey: 'local'),
              ],
            ),
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();
    choices.add(null);
    await tester.pumpAndSettle();
    expect(find.text('Delete'), findsOneWidget);
    await tester.tap(find.text('Delete'));
    await tester.pumpAndSettle();
    expect(find.text('Delete Scout?'), findsOneWidget);
    await tester.tap(find.text('Cancel'));
    await tester.pumpAndSettle();
    expect(h.registry.calls, isEmpty);
    expect(controller.agents, hasLength(1));
    choices.add(agent);
    await tester.pumpAndSettle();
    expect(
      tester.widget<FilledButton>(find.byType(FilledButton)).onPressed,
      isNull,
    );
    expect(
      find.text('Change your Bestie on desktop before deleting this agent.'),
      findsOneWidget,
    );
    // Desktop changes the selection while the confirmation is open.
    choices.add(null);
    await tester.pumpAndSettle();
    await tester.tap(find.text('Delete'));
    await tester.pumpAndSettle();
    await saveDefaultAgent(sync, agent);
    await tester.tap(find.widgetWithText(TextButton, 'Delete'));
    await tester.pumpAndSettle();
    expect(h.registry.calls, isEmpty);
    expect(controller.agents, hasLength(1));
    expect(
      find.text('Change your Bestie on desktop before deleting this agent.'),
      findsOneWidget,
    );
  });
}
