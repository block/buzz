import 'dart:convert';

import 'package:buzz/features/agents/agent_creation_draft.dart';
import 'package:buzz/features/agents/agents_page.dart';
import 'package:buzz/features/agents/create_agent_page.dart';
import 'package:buzz/shared/deeplink/deep_link.dart';
import 'package:buzz/shared/hosted/hosted_environment.dart';
import 'package:buzz/shared/hosted/hosted_flow.dart';
import 'package:buzz/shared/hosted/hosted_flow_provider.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';
import 'package:shared_preferences/shared_preferences.dart';

import '../../helpers/remote_agents_fixture.dart';
import 'agents_harness.dart';

/// Fakes only the browser sign-in. Like the real flow, completion clears
/// `reconnecting` in the same synchronous step that reports `complete`.
class _SignInFlow extends ChangeNotifier implements HostedFlow {
  @override
  HostedFlowStage stage = HostedFlowStage.idle;
  @override
  bool reconnecting = false;
  int starts = 0;

  @override
  Future<void> reconnectAccount({bool changeAccount = false}) async {
    starts++;
    reconnecting = true;
    stage = HostedFlowStage.signIn;
    notifyListeners();
  }

  void complete() {
    stage = HostedFlowStage.complete;
    reconnecting = false;
    notifyListeners();
  }

  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}

void main() {
  testWidgets(
    'a link that needs sign-in still respects a saved unfinished setup',
    (tester) async {
      final h = await AgentsHarness.create();
      // The login expired, but this account saved an unfinished setup.
      h.registry.signedIn = false;
      final prefs = await SharedPreferences.getInstance();
      await prefs.setString(
        'remote-agents:${HostedEnvironment.origin.host}:$fakeSubject:pending',
        jsonEncode({
          'name': 'Saved',
          'instructions': '',
          'community': h.community.relayUrl,
          'owner': fakeOwner.public,
          'request': 'saved-request',
        }),
      );
      final flow = _SignInFlow();
      var cardFetches = 0;
      await h.open(
        tester,
        directory: const [],
        overrides: [
          hostedFlowProvider.overrideWithValue(flow),
          agentCardClientProvider.overrideWithValue(
            () => MockClient((_) async {
              cardFetches++;
              return http.Response('{}', 404);
            }),
          ),
        ],
        home: Builder(
          builder: (context) => Scaffold(
            body: TextButton(
              onPressed: () => showAgents(
                context,
                draft: const AgentCreationDeepLink('av_card'),
              ),
              child: const Text('Open link'),
            ),
          ),
        ),
      );
      await tester.tap(find.text('Open link'));
      await tester.pumpAndSettle();
      expect(flow.starts, 1);

      // Sign-in lands. The registry is reloaded for this account before the
      // card is considered, so the saved setup blocks it.
      h.registry.signedIn = true;
      flow.complete();
      await tester.pumpAndSettle();
      expect(find.byType(CreateAgentPage), findsNothing);
      expect(cardFetches, 0);
      expect(
        find.text(
          'Finish your unfinished agent setup first, then reopen the link.',
        ),
        findsOneWidget,
      );
      expect(h.controller.pending?['name'], 'Saved');
    },
    skip: !HostedEnvironment.enabled,
  );
}
