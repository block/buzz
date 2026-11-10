import 'package:buzz/features/agents/delete_agent_action.dart';
import 'package:buzz/shared/community/community_provider.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/material.dart';
import 'dart:async';
import 'package:buzz/shared/widgets/app_list.dart';
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
    final agent = nostr.Keys('0' * 63 + '2').public;
    h.registry.listed = [
      {'agent_pubkey': agent, 'agent_id': 'remote-id', 'agent_name': 'Scout'},
    ];
    await controller.run(controller.refresh);
    h.registry.calls.clear();
    await tester.pumpWidget(
      ProviderScope(
        overrides: [
          activeCommunityProvider.overrideWith((ref) async => h.community),
        ],
        child: MaterialApp(
          theme: AppTheme.light(),
          home: Scaffold(
            body: Column(
              children: [
                DeleteAgentAction(
                  controller: controller,
                  pubkey: agent,
                  asSettingsRow: true,
                ),
                DeleteAgentAction(controller: controller, pubkey: 'local'),
              ],
            ),
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();
    expect(find.text('Delete agent'), findsOneWidget);
    final refresh = Completer<void>();
    final refreshing = controller.run(() => refresh.future);
    await tester.pump();
    expect(find.text('Deleting…'), findsNothing);
    expect(find.text('Delete agent'), findsOneWidget);
    expect(tester.widget<AppListRow>(find.byType(AppListRow)).onTap, isNull);
    refresh.complete();
    await refreshing;
    await tester.pumpAndSettle();
    await tester.tap(find.text('Delete agent'));
    await tester.pumpAndSettle();
    expect(find.text('Delete Scout?'), findsOneWidget);
    await tester.tap(find.text('Cancel'));
    await tester.pumpAndSettle();
    expect(h.registry.calls, isEmpty);
    expect(controller.agents, hasLength(1));
  });
}
