import 'package:buzz/features/agents/agent_identity_sections.dart';
import 'package:buzz/shared/community/community_provider.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;

import 'agents_harness.dart';

void main() {
  testWidgets(
    'Edit instructions appears for owned active remote agent and shows instructions',
    (tester) async {
      final h = await AgentsHarness.create();
      final controller = h.controller;
      final agent = nostr.Keys('0' * 63 + '2').public;
      h.registry.listed = [
        {
          'agent_pubkey': agent,
          'agent_id': 'remote-id',
          'agent_name': 'Scout',
          'status': 2,
          'agent_instructions': 'Follow these scout instructions carefully.',
        },
      ];
      await controller.run(controller.refresh);

      await tester.pumpWidget(
        ProviderScope(
          overrides: [
            activeCommunityProvider.overrideWith((ref) async => h.community),
          ],
          child: MaterialApp(
            theme: AppTheme.light(),
            home: Scaffold(
              body: SingleChildScrollView(
                child: AgentIdentitySections(
                  controller: controller,
                  pubkey: agent,
                ),
              ),
            ),
          ),
        ),
      );
      await tester.pumpAndSettle();

      expect(
        find.text('Follow these scout instructions carefully.'),
        findsOneWidget,
      );
      expect(
        find.byKey(const ValueKey('edit-agent-instructions-row')),
        findsOneWidget,
      );
      expect(find.text('Edit instructions'), findsOneWidget);
    },
  );

  testWidgets(
    'Edit instructions does not appear when agent is not owned or not remote',
    (tester) async {
      final h = await AgentsHarness.create();
      final controller = h.controller;
      final foreignAgent = nostr.Keys('0' * 63 + '9').public;

      await tester.pumpWidget(
        ProviderScope(
          overrides: [
            activeCommunityProvider.overrideWith((ref) async => h.community),
          ],
          child: MaterialApp(
            theme: AppTheme.light(),
            home: Scaffold(
              body: SingleChildScrollView(
                child: AgentIdentitySections(
                  controller: controller,
                  pubkey: foreignAgent,
                ),
              ),
            ),
          ),
        ),
      );
      await tester.pumpAndSettle();

      expect(
        find.byKey(const ValueKey('edit-agent-instructions-row')),
        findsNothing,
      );
    },
  );
}
