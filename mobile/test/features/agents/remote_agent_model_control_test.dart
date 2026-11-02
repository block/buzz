import 'dart:async';

import 'package:buzz/features/agents/remote_agent_model_control.dart';
import 'package:buzz/shared/community/community.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../helpers/remote_agents_fixture.dart';

void main() {
  testWidgets('cards share catalog loading, retry, and account transitions', (
    tester,
  ) async {
    tester.view.physicalSize = const Size(390, 1600);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.resetPhysicalSize);
    addTearDown(tester.view.resetDevicePixelRatio);
    final registry = FakeRegistry();
    final controller = RelayFakeController(registry);
    addTearDown(() {
      controller.dispose();
      controller.relayAuth.dispose();
      registry.dispose();
    });
    final community = Community.create(
      name: 'Test',
      relayUrl: 'wss://test.example',
    );
    final gate = Completer<void>();
    registry.catalogGate = gate;
    int requests() =>
        registry.calls.where((call) => call == 'get-model-catalog').length;
    Future<void> render(int count) => tester.pumpWidget(
      ProviderScope(
        child: MaterialApp(
          theme: AppTheme.light(),
          home: Scaffold(
            body: Column(
              children: [
                for (var i = 0; i < count; i++)
                  RemoteAgentModelControl(
                    controller: controller,
                    agent: {'agent_pubkey': '$i', 'model_id': 'test-model'},
                    community: community,
                    isCurrent: () => true,
                  ),
              ],
            ),
          ),
        ),
      ),
    );
    await render(15);
    await tester.pump();
    expect(requests(), 1);
    expect(find.byType(CircularProgressIndicator), findsNWidgets(15));
    gate.completeError(StateError('Unavailable'));
    await tester.pumpAndSettle();
    expect(find.text('Retry models'), findsNWidgets(15));
    expect(requests(), 1);

    registry.catalogGate = null;
    await tester.tap(find.text('Retry models').first);
    await tester.pumpAndSettle();
    expect(requests(), 2);
    expect(find.text('Test model'), findsNWidgets(15));
    // A later-mounted card uses the same result, not another queued call.
    await render(16);
    await tester.pumpAndSettle();
    expect(requests(), 2);
    expect(find.text('Test model'), findsNWidgets(16));

    final next = Completer<void>();
    registry.catalogGate = next;
    registry.subject = 'another-owner';
    await tester.pump();
    await tester.pump();
    expect(requests(), 3);
    expect(find.text('Test model'), findsNothing);
    expect(find.byType(CircularProgressIndicator), findsNWidgets(16));
    // Signing out while that request is pending cannot restore its result.
    registry.signedIn = false;
    await tester.pumpAndSettle();
    expect(find.text('Retry models'), findsNWidgets(16));
    next.complete();
    await tester.pumpAndSettle();
    expect(find.text('Test model'), findsNothing);
    expect(requests(), 3);

    registry.catalogGate = null;
    registry.signedIn = true;
    await tester.pumpAndSettle();
    expect(requests(), 4);
    expect(find.text('Test model'), findsNWidgets(16));
    expect(tester.takeException(), isNull);
  });
}
