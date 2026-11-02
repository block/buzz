import 'package:buzz/features/agents/edit_agent_profile_page.dart';
import 'package:buzz/shared/community/community_provider.dart';
import 'package:buzz/shared/agents/remote_agents.dart';
import 'package:buzz/shared/hosted/hosted_environment.dart';
import 'package:buzz/shared/profile/user_cache_provider.dart';
import 'package:buzz/shared/profile/user_profile.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../helpers/remote_agents_fixture.dart';
import 'agents_harness.dart';

void main() {
  testWidgets(
    'owners rename an active remote agent from its page; others see nothing',
    (tester) async {
      final h = await AgentsHarness.create();
      final controller = h.controller;
      h.registry.listed = [
        {
          'agent_pubkey': fakeAgent,
          'agent_id': 'remote-id',
          'agent_name': 'Scout',
          'status': 2,
        },
      ];
      await controller.run(controller.refresh);
      await tester.pumpWidget(
        ProviderScope(
          overrides: [
            activeCommunityProvider.overrideWith((ref) async => h.community),
            relaySessionProvider.overrideWith(() => h.relay),
          ],
          child: MaterialApp(
            theme: AppTheme.light(),
            home: Scaffold(
              body: Column(
                children: [
                  EditAgentProfileAction(
                    controller: controller,
                    pubkey: fakeAgent,
                  ),
                  // Not this owner's active remote agent.
                  EditAgentProfileAction(
                    controller: controller,
                    pubkey: 'local',
                  ),
                ],
              ),
            ),
          ),
        ),
      );
      await tester.pumpAndSettle();
      // Hidden until the sheet has loaded the agent's profile.
      expect(find.text('Edit name and avatar'), findsNothing);
      ProviderScope.containerOf(tester.element(find.byType(Scaffold)))
          .read(userCacheProvider.notifier)
          .put(
            UserProfile(
              pubkey: fakeAgent,
              displayName: 'Scout',
              avatarUrl: 'https://example.com/kept.png',
            ),
          );
      await tester.pumpAndSettle();
      expect(find.text('Edit name and avatar'), findsOneWidget);

      await tester.tap(find.text('Edit name and avatar'));
      await tester.pumpAndSettle();
      final save = find.widgetWithIcon(IconButton, Icons.check);
      // Nothing changed yet.
      expect(tester.widget<IconButton>(save).onPressed, isNull);
      // Invalid names are caught before anything is uploaded or sent.
      await tester.enterText(find.byType(TextField), 'no/slash');
      await tester.pump();
      await tester.tap(save);
      await tester.pumpAndSettle();
      expect(find.text(invalidAgentName), findsOneWidget);
      expect(h.registry.bodies['/v3/beekeeper/update-agent-profile'], isNull);

      await tester.enterText(find.byType(TextField), 'Nova');
      await tester.pump();
      await tester.tap(save);
      await tester.pumpAndSettle();

      expect(h.registry.bodies['/v3/beekeeper/update-agent-profile'], {
        'agent_pubkey': fakeAgent,
        'community_url': 'wss://test.${HostedEnvironment.communitySuffix}',
        'agent_name': 'Nova',
        // An unchanged avatar is kept by Builderlab.
        'agent_picture_url': '',
      });
      expect(controller.published.single['name'], 'Nova');
      expect(find.text('Edit agent'), findsNothing);
      expect(find.text('Agent updated'), findsOneWidget);
      expect(tester.takeException(), isNull);
    },
  );
}
