import 'dart:async';
import 'package:buzz/shared/hosted/hosted_flow.dart';
import 'package:buzz/features/profile/profile_setup_page.dart';
import 'package:flutter/material.dart';
import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:flutter_test/flutter_test.dart';

import '../../helpers/hosted_communities_fixture.dart';
import '../../helpers/widget_helpers.dart';

void main() {
  testWidgets('failed fallback save can continue without saving', (
    tester,
  ) async {
    FlutterSecureStorage.setMockInitialValues({});
    final fixture = HostedCommunitiesFixture();
    await fixture.initialize(widget: true);
    addTearDown(fixture.dispose);
    var attempts = 0;
    final release = Completer<void>();
    final pending = fixture.flow.requestProfile((_) async {
      attempts++;
      await release.future;
      throw StateError('Relay unavailable');
    });
    await tester.pumpWidget(
      WidgetHelpers.testable(child: ProfileSetupPage(flow: fixture.flow)),
    );
    await tester.pumpAndSettle();
    expect(find.text('Not now'), findsNothing);
    expect(find.byTooltip('Back'), findsNothing);
    await tester.binding.handlePopRoute();
    await tester.pump();
    expect(fixture.flow.stage, HostedFlowStage.profile);
    await tester.enterText(find.byType(TextField), 'Kenny');
    await tester.pump();
    await tester.tap(find.text('Continue'));
    await tester.pump();
    expect(find.text('Saving…'), findsOneWidget);
    await tester.binding.handlePopRoute();
    await tester.pump();
    expect(fixture.flow.stage, HostedFlowStage.profile);
    expect(find.text('Saving…'), findsOneWidget);
    release.complete();
    await tester.pumpAndSettle();
    expect(attempts, 1);
    expect(find.text('Not now'), findsOneWidget);
    await tester.tap(find.text('Not now'));
    await tester.pumpAndSettle();
    await pending;
    expect(attempts, 1);
    await tester.pumpWidget(const SizedBox.shrink());
    await tester.pumpAndSettle();
  }, variant: TargetPlatformVariant.only(TargetPlatform.android));
}
