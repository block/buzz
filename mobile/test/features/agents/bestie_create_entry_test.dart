import 'dart:convert';
import 'package:buzz/shared/profile/profile_event_parser.dart';
import 'package:buzz/shared/profile/user_cache_provider.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/features/channels/default_agent_launcher.dart';
import 'package:buzz/shared/agents/first_agent_bestie.dart';
import 'package:buzz/shared/me/me_sync.dart';
import 'package:buzz/shared/me/default_agent.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:buzz/features/agents/create_agent_page.dart';
import 'package:buzz/features/agents/agents_page.dart';
import 'package:buzz/shared/hosted/hosted_environment.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'agents_harness.dart';

void main() {
  testWidgets(
    'Bestie launcher creates a first agent, saves the choice and returns home',
    (tester) async {
      late AgentsHarness h;
      late ProviderContainer container;
      h = await AgentsHarness.create(
        onCreated: (community, pubkey) async {
          // The fake relay exposes the registration published by the service.
          h.registered = [pubkey];
          await makeFirstAgentBestie(container.read(meSyncProvider), pubkey);
          container.invalidate(defaultAgentProvider);
        },
      );
      await h.open(
        tester,
        directory: [],
        overrides: [
          // Keep verification real, but avoid an isolate outside fake async.
          profileEventBatchParserProvider.overrideWithValue(
            (events) async => events.map(parseProfileEvent).toList(),
          ),
        ],
        home: Builder(
          builder: (context) => Scaffold(
            body: DefaultAgentLauncher(
              onChooseAgent: () => showAgents(context, createOnOpen: true),
            ),
          ),
        ),
      );
      container = ProviderScope.containerOf(
        tester.element(find.byType(DefaultAgentLauncher)),
      );
      await tester.tap(find.byTooltip('Bestie'));
      await tester.pumpAndSettle();
      expect(find.byType(CreateAgentPage), findsOneWidget);
      expect(find.byType(AgentsPage, skipOffstage: false), findsOneWidget);
      await tester.tap(find.byTooltip('Cancel'));
      await tester.pumpAndSettle();
      expect(find.byType(CreateAgentPage), findsNothing);
      expect(find.byType(AgentsPage), findsOneWidget);
      await tester.tap(find.byTooltip('Close agents'));
      await tester.pumpAndSettle();
      expect(find.byTooltip('Bestie'), findsOneWidget);
      await tester.tap(find.byTooltip('Bestie'));
      await tester.pumpAndSettle();
      await tester.enterText(find.byType(TextField).first, 'Scout');
      await tester.tap(find.text('Model'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Test model').last);
      await tester.pumpAndSettle();
      await tester.tap(find.text('Thinking'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('high').last);
      await tester.pumpAndSettle();
      await tester.tap(find.byTooltip('Create'));
      await tester.pumpAndSettle();
      expect(find.byType(CreateAgentPage), findsNothing);
      await tester.tap(find.byTooltip('Close agents'));
      await tester.pumpAndSettle();
      expect(container.read(defaultAgentProvider).value, h.registered.single);
      await tester.pumpAndSettle();
      expect(find.byTooltip('Chat with Scout'), findsOneWidget);
      final sync = container.read(meSyncProvider);
      final saved = sync.prefs!.getString(
        'main-agent:${sync.community}:${sync.pubkey}',
      );
      expect(
        decodeDefaultAgent(NostrEvent.fromJson(jsonDecode(saved!)), sync),
        h.registered.single,
      );
      await tester.pumpWidget(const SizedBox());
    },
    skip: !HostedEnvironment.enabled,
  );
}
