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
import 'package:buzz/features/channels/bestie_welcome.dart';
import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channel_detail_page.dart';
import 'package:buzz/features/channels/channel_management_provider.dart';
import 'package:buzz/shared/hosted/hosted_environment.dart';
import 'package:buzz/shared/theme/theme_provider.dart';
import 'package:flutter/material.dart';
import 'package:shared_preferences/shared_preferences.dart';
import 'package:flutter_test/flutter_test.dart';

import 'agents_harness.dart';

void main() {
  testWidgets(
    'Bestie launcher creates a first agent, saves the choice and opens its chat',
    (tester) async {
      final dms = _DmActions();
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
      final prefs = await SharedPreferences.getInstance();
      await h.open(
        tester,
        directory: [],
        overrides: [
          channelActionsProvider.overrideWithValue(dms),
          savedPrefsProvider.overrideWithValue(prefs),
          // Keep verification real, but avoid an isolate outside fake async.
          profileEventBatchParserProvider.overrideWithValue(
            (events) async => events.map(parseProfileEvent).toList(),
          ),
        ],
        // Like iOS Home, drop the shortcut while another route covers it, so
        // the context that opened Agents is gone when creation finishes.
        home: Builder(
          builder: (context) => Scaffold(
            body: ModalRoute.of(context)?.isCurrent ?? true
                ? Builder(
                    builder: (shortcut) => DefaultAgentLauncher(
                      onChooseAgent: () =>
                          showAgents(shortcut, createOnOpen: true),
                    ),
                  )
                : const SizedBox(),
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
      // A successful first creation leaves Agents for the new Bestie's chat.
      expect(find.byType(CreateAgentPage), findsNothing);
      expect(find.byType(AgentsPage, skipOffstage: false), findsNothing);
      expect(find.byType(ChannelDetailPage), findsOneWidget);
      expect(find.byType(BestieWelcome), findsOneWidget);
      expect(dms.opened, [
        [h.registered.single],
      ]);
      expect(container.read(defaultAgentProvider).value, h.registered.single);
      final sync = container.read(meSyncProvider);
      final saved = sync.prefs!.getString(
        'main-agent:${sync.community}:${sync.pubkey}',
      );
      expect(
        decodeDefaultAgent(NostrEvent.fromJson(jsonDecode(saved!)), sync),
        h.registered.single,
      );
      await tester.pumpWidget(const SizedBox());
      // Let the closed chat's delayed work finish before teardown.
      await tester.pump(const Duration(minutes: 1));
    },
    skip: !HostedEnvironment.enabled,
  );
}

class _DmActions extends Fake implements ChannelActions {
  final opened = <List<String>>[];

  @override
  Future<Channel> openDm({
    required List<String> pubkeys,
    bool waitForChannelList = true,
  }) async {
    opened.add(pubkeys);
    return Channel(
      id: 'bestie-dm',
      name: 'Scout',
      channelType: 'dm',
      visibility: 'private',
      description: '',
      createdBy: 'self',
      createdAt: DateTime(2026),
      memberCount: 2,
      participantPubkeys: pubkeys,
      isMember: true,
    );
  }
}
