import 'package:buzz/features/agents/agents_page.dart';
import 'package:buzz/features/agents/create_agent_page.dart';
import 'package:buzz/shared/hosted/hosted_environment.dart';
import 'package:buzz/shared/hosted/hosted_flow.dart';
import 'package:buzz/shared/hosted/hosted_flow_provider.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'agents_harness.dart';

/// Fakes only the browser sign-in. Like the real flow, completion clears
/// `reconnecting` in the same synchronous step that reports `complete`.
class _SignInFlow extends ChangeNotifier implements HostedFlow {
  @override
  HostedFlowStage stage = HostedFlowStage.idle;
  @override
  bool reconnecting = false;
  int starts = 0;
  bool fail = false;

  @override
  Future<void> reconnectAccount({bool changeAccount = false}) async {
    starts++;
    if (fail) throw StateError('Browser unavailable');
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
    'Bestie creation reports sign-in failure and resumes after sign-in',
    (tester) async {
      final h = await AgentsHarness.create();
      h.registry.signedIn = false;
      final flow = _SignInFlow()..fail = true;
      addTearDown(flow.dispose);
      await h.open(
        tester,
        directory: [],
        createOnOpen: true,
        overrides: [hostedFlowProvider.overrideWithValue(flow)],
      );
      expect(
        find.text('Couldn’t start Builderlab sign-in. Reopen Agents.'),
        findsOneWidget,
      );
      expect(tester.takeException(), isNull);
      await tester.tap(find.byTooltip('Close agents'));
      await tester.pumpAndSettle();
      flow.fail = false;
      await tester.tap(find.text('Open Agents'));
      // The indeterminate progress indicator deliberately stays active while
      // the browser owns sign-in, so wait only for the route animation.
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 400));
      expect(flow.starts, 2);
      expect(find.byType(LinearProgressIndicator), findsOneWidget);
      expect(find.byType(CreateAgentPage), findsNothing);
      final context = tester.element(
        find.text('Open Agents', skipOffstage: false),
      );
      showAgents(context, createOnOpen: true);
      await tester.pump();
      expect(flow.starts, 2);
      h.registry.signedIn = true;
      flow.complete();
      await tester.pumpAndSettle();
      expect(find.byType(CreateAgentPage), findsOneWidget);
      expect(tester.takeException(), isNull);
      await tester.pumpWidget(const SizedBox());
    },
    skip: !HostedEnvironment.enabled,
  );
}
