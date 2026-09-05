import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:buzz/shared/agent_usage/agent_usage_indicator.dart';
import 'package:buzz/shared/agent_usage/agent_usage_models.dart';
import 'package:buzz/shared/agent_usage/agent_usage_subscription.dart';

import '../../helpers/widget_helpers.dart';

const _agentPubkey = 'deadbeef';

void main() {
  Widget buildIndicator(AgentUsageRelayState fixedState) {
    return WidgetHelpers.testable(
      overrides: [
        agentUsageRelayProvider.overrideWith(
          () => _FixedAgentUsageRelayNotifier(fixedState),
        ),
      ],
      child: const AgentUsageIndicator(
        agentPubkey: _agentPubkey,
        agentLabel: 'Test Bot',
        child: CircleAvatar(radius: 20, child: Text('T')),
      ),
    );
  }

  testWidgets('shows an "unknown" state with no ring when no data exists', (
    tester,
  ) async {
    await tester.pumpWidget(
      buildIndicator(
        const AgentUsageRelayState(
          connection: AgentUsageConnectionState.open,
          snapshotsByAgent: {},
        ),
      ),
    );

    expect(find.bySemanticsLabel('Agent usage: no data yet'), findsOneWidget);
  });

  testWidgets(
    'shows an "error" state when the subscription is unhealthy and no data exists',
    (tester) async {
      await tester.pumpWidget(
        buildIndicator(
          const AgentUsageRelayState(
            connection: AgentUsageConnectionState.error,
            snapshotsByAgent: {},
            errorMessage: 'boom',
          ),
        ),
      );

      expect(find.bySemanticsLabel('Agent usage: unavailable'), findsOneWidget);
    },
  );

  testWidgets('shows the usage percentage when a fresh snapshot exists', (
    tester,
  ) async {
    await tester.pumpWidget(
      buildIndicator(
        AgentUsageRelayState(
          connection: AgentUsageConnectionState.open,
          snapshotsByAgent: {
            _agentPubkey: AgentUsageSnapshot(
              harness: 'goose',
              lastEventAt: DateTime.now(),
              contextUsedTokens: 50,
              contextLimitTokens: 100,
            ),
          },
        ),
      ),
    );

    expect(find.bySemanticsLabel('Agent usage: 50% used'), findsOneWidget);
  });

  testWidgets('flags stale data once it exceeds the freshness window', (
    tester,
  ) async {
    await tester.pumpWidget(
      buildIndicator(
        AgentUsageRelayState(
          connection: AgentUsageConnectionState.open,
          snapshotsByAgent: {
            _agentPubkey: AgentUsageSnapshot(
              harness: 'goose',
              lastEventAt: DateTime.now().subtract(const Duration(days: 2)),
              contextUsedTokens: 50,
              contextLimitTokens: 100,
            ),
          },
        ),
      ),
    );

    expect(
      find.bySemanticsLabel('Agent usage: 50%, data may be out of date'),
      findsOneWidget,
    );
  });

  testWidgets('tapping opens the detail sheet with the agent label as title', (
    tester,
  ) async {
    await tester.pumpWidget(
      buildIndicator(
        const AgentUsageRelayState(
          connection: AgentUsageConnectionState.open,
          snapshotsByAgent: {},
        ),
      ),
    );

    await tester.tap(find.bySemanticsLabel('Agent usage: no data yet'));
    await tester.pumpAndSettle();

    expect(find.text('Test Bot'), findsOneWidget);
    expect(find.text('No usage data reported yet.'), findsOneWidget);
  });
}

class _FixedAgentUsageRelayNotifier extends AgentUsageRelayNotifier {
  final AgentUsageRelayState fixedState;

  _FixedAgentUsageRelayNotifier(this.fixedState);

  @override
  AgentUsageRelayState build() => fixedState;
}
