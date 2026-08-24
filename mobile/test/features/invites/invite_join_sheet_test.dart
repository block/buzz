import 'package:buzz/features/invites/invite_join_provider.dart';
import 'package:buzz/features/invites/invite_join_sheet.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import '../../helpers/widget_helpers.dart';

void main() {
  testWidgets('recovery error is scrollable and exposes retry setup', (
    tester,
  ) async {
    tester.view.physicalSize = const Size(375, 400);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.reset);

    await tester.pumpWidget(
      WidgetHelpers.testable(
        child: const InviteJoinSheet(),
        overrides: [
          inviteJoinProvider.overrideWith(_RecoveryErrorInviteJoinNotifier.new),
        ],
      ),
    );
    await tester.pump();

    expect(find.text('Finish setting up'), findsOneWidget);
    expect(find.widgetWithText(FilledButton, 'Retry setup'), findsOneWidget);
    expect(find.byType(SingleChildScrollView), findsOneWidget);
    expect(tester.takeException(), isNull);
  });
}

class _RecoveryErrorInviteJoinNotifier extends InviteJoinNotifier {
  @override
  InviteJoinState build() => const InviteJoinState(
    status: InviteJoinStatus.error,
    host: 'relay.example.com',
    communityName: 'Example',
    errorMessage:
        'Starter setup could not reach the relay. Retry when the connection is available.',
    isStarterSetupRecovery: true,
  );
}
