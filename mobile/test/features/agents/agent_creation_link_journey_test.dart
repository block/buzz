import 'dart:async';
import 'dart:convert';

import 'package:buzz/features/agents/agent_creation_draft.dart';
import 'package:buzz/features/agents/agents_page.dart';
import 'package:buzz/main.dart';
import 'package:buzz/shared/hosted/hosted_environment.dart';
import 'package:buzz/features/agents/create_agent_page.dart';
import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channels_provider.dart';
import 'package:buzz/features/channels/deep_link_dispatcher.dart';
import 'package:buzz/shared/agents/remote_agents.dart';
import 'package:buzz/shared/deeplink/pending_deep_link_provider.dart';
import 'package:buzz/shared/widgets/preset_agent_avatar.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';
import 'package:shared_preferences/shared_preferences.dart';

import 'agents_harness.dart';

class _NoChannels extends ChannelsNotifier {
  @override
  Future<List<Channel>> build() async => const [];
}

const _cdn = 'https://d2z0pbxrxbsiz5.cloudfront.net';

String _link(String card) => 'buzz://agent/new?card=$card';

/// Published `agent.json` files on the avatars CDN, by card id.
final _cards = <String, Map<String, Object?>>{
  'av_full': {
    'v': 1,
    'id': 'av_full',
    'name': 'Scout',
    'avatarUrl': '$_cdn/av_full/r1/avatar.png',
    'instructions': 'Reply concisely.',
    'model': 'test-model',
    'reasoningEffort': 'high',
  },
  // A card as published today: no instructions or model.
  'av_v1': {
    'v': 1,
    'id': 'av_v1',
    'collection': 'gloopie',
    'name': 'Owl',
    'avatarUrl': '$_cdn/av_v1/r1/avatar.png',
    'model': 'retired-model',
  },
  'av_third': {
    'v': 1,
    'id': 'av_third',
    'name': 'Third',
    'avatarUrl': '$_cdn/av_third/r1/avatar.png',
  },
  'av_fourth': {
    'v': 1,
    'id': 'av_fourth',
    'name': 'Fourth',
    'avatarUrl': '$_cdn/av_fourth/r1/avatar.png',
  },
};

/// Fakes only the CDN: unknown ids get S3's 403.
final _cdnRequests = <Uri>[];
http.Client _cdnClient() => MockClient((request) async {
  _cdnRequests.add(request.url);
  final match = RegExp(r'^/([^/]+)/agent\.json$').firstMatch(request.url.path);
  final card = request.url.origin == _cdn ? _cards[match?.group(1)] : null;
  return card == null
      ? http.Response('<Error>AccessDenied</Error>', 403)
      : http.Response(jsonEncode(card), 200);
});

Finder _field(String label) => find.descendant(
  of: find.byWidgetPredicate(
    (w) => w is Semantics && w.properties.label == label,
  ),
  matching: find.byType(TextField),
);

void main() {
  testWidgets(
    'a buzz://agent/new?card= link opens an editable draft, never creates',
    (tester) async {
      final h = await AgentsHarness.create();
      expect(supportsRemoteAgents(h.community), isTrue);
      final links = StreamController<Uri>();
      PendingDeepLinkNotifier.debugUriStreamOverride = links.stream;
      addTearDown(() async {
        PendingDeepLinkNotifier.debugUriStreamOverride = null;
        await links.close();
      });
      _cdnRequests.clear();
      await h.open(
        tester,
        directory: const [],
        overrides: [
          channelsProvider.overrideWith(_NoChannels.new),
          agentCardClientProvider.overrideWithValue(_cdnClient),
          // The production root wiring from link dispatch to showAgents.
          ...buildRootProviderScope(
            prefs: await SharedPreferences.getInstance(),
            child: const SizedBox(),
          ).overrides,
        ],
        home: const DeepLinkDispatcher(child: Scaffold(body: SizedBox())),
      );
      Future<void> receive(String url) async {
        links.add(Uri.parse(url));
        await tester.pumpAndSettle();
      }

      // A malformed link, or the retired inline form, opens nothing.
      await receive('buzz://agent/new?card=..');
      await receive('buzz://agent/new#e30');
      expect(find.byType(AgentsPage), findsNothing);
      expect(_cdnRequests, isEmpty);

      // A card lands on Agents, then a form prefilled from its agent.json,
      // fetched from this build's CDN.
      await receive(_link('av_full'));
      expect(_cdnRequests.single.toString(), '$_cdn/av_full/agent.json');
      expect(find.byType(CreateAgentPage), findsOneWidget);
      expect(find.text('Scout'), findsOneWidget);
      expect(find.text('Reply concisely.'), findsOneWidget);
      expect(find.text('Test model'), findsOneWidget);
      expect(find.text('high'), findsOneWidget);
      expect(
        tester
            .widget<PresetAgentAvatar>(find.byType(PresetAgentAvatar))
            .imageUrl,
        '$_cdn/av_full/r1/avatar.png',
      );
      expect(h.registry.calls, isNot(contains('register-agent')));

      // Everything stays editable, then Create submits the edited values.
      await tester.enterText(_field('Name'), 'Scout Two');
      await tester.enterText(_field('Instructions'), 'Be formal.');
      await tester.ensureVisible(find.text('Thinking'));
      await tester.tap(find.text('Thinking'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('low').last);
      await tester.pumpAndSettle();
      await tester.tap(find.byTooltip('Create'));
      await tester.pumpAndSettle();
      expect(h.controller.submittedName, 'Scout Two');
      expect(h.controller.submittedInstructions, 'Be formal.');
      final registered = h.registry.bodies['/v3/beekeeper/register-agent']!;
      expect(registered['agent_name'], 'Scout Two');
      expect(registered['agent_picture_url'], '$_cdn/av_full/r1/avatar.png');
      expect(
        h.registry.bodies['/v3/beekeeper/update-agent'],
        containsPair('model_id', 'test-model'),
      );
      expect(
        h.registry.bodies['/v3/beekeeper/update-agent'],
        containsPair('reasoning_effort', 'low'),
      );
      expect(find.byType(CreateAgentPage), findsNothing);
      await tester.tap(find.byTooltip('Close agents'));
      await tester.pumpAndSettle();

      // A card that is gone shows why and opens no form.
      await receive(_link('av_missing'));
      expect(find.byType(CreateAgentPage), findsNothing);
      expect(find.text('This agent card isn’t available.'), findsOneWidget);
      await tester.tap(find.byTooltip('Close agents'));
      await tester.pumpAndSettle();

      // A card as published today (no instructions, a model the catalog
      // lacks): the draft opens with empty instructions
      // and the model left for the user, so Create stays disabled.
      await receive(_link('av_v1'));
      expect(find.byType(CreateAgentPage), findsOneWidget);
      expect(find.text('Owl'), findsOneWidget);
      expect(
        tester.widget<TextField>(_field('Instructions')).controller!.text,
        isEmpty,
      );
      expect(find.text('Retired model'), findsNothing);
      expect(find.text('Choose'), findsNWidgets(2));
      expect(
        tester
            .widget<IconButton>(find.widgetWithIcon(IconButton, Icons.check))
            .onPressed,
        isNull,
      );

      // Pick a model by hand and fail registration to leave a setup unfinished.
      h.registry.loseRegistration = true;
      await tester.ensureVisible(find.text('Model'));
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
      expect(h.controller.pending, isNotNull);
      expect(
        h.registry.bodies['/v3/beekeeper/register-agent']!['agent_picture_url'],
        '$_cdn/av_v1/r1/avatar.png',
      );
      await tester.tap(find.byTooltip('Cancel'));
      await tester.pumpAndSettle();
      await tester.tap(find.byTooltip('Close agents'));
      await tester.pumpAndSettle();

      // With an unfinished setup, a new link must not open a draft.
      await receive(_link('av_third'));
      expect(find.byType(CreateAgentPage), findsNothing);
      expect(find.byType(AgentsPage), findsOneWidget);
      expect(
        find.textContaining('Finish your unfinished agent setup'),
        findsOne,
      );

      // A link that arrives while Agents is open replaces it, not stacks.
      await receive(_link('av_fourth'));
      expect(find.byType(AgentsPage, skipOffstage: false), findsOneWidget);
    },
    // Remote agents exist only in Builderlab builds; CI runs this file in its
    // configured-authentication-domains step.
    skip: !HostedEnvironment.enabled,
  );
}
