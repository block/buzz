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

  for (final fixture in [
    (
      name: 'membership claim',
      state: const InviteJoinState(
        status: InviteJoinStatus.claiming,
        host: 'relay.example.com',
      ),
      label: 'Joining…',
    ),
    (
      name: 'starter recovery',
      state: const InviteJoinState(
        status: InviteJoinStatus.claiming,
        host: 'relay.example.com',
        isStarterSetupRecovery: true,
      ),
      label: 'Finishing setup…',
    ),
  ]) {
    testWidgets('${fixture.name} cannot dismiss the in-flight invite sheet', (
      tester,
    ) async {
      await tester.pumpWidget(
        WidgetHelpers.testable(
          child: const _InviteJoinSheetLauncher(),
          overrides: [
            inviteJoinProvider.overrideWith(
              () => _StaticInviteJoinNotifier(fixture.state),
            ),
          ],
        ),
      );

      await tester.tap(find.text('Open invite'));
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 300));
      expect(find.text(fixture.label), findsOneWidget);
      expect(
        find.byKey(const ValueKey('buzz-sheet-drag-handle')),
        findsNothing,
      );
      expect(find.byTooltip('Close sheet'), findsNothing);

      await tester.tapAt(const Offset(8, 8));
      await tester.pump();
      expect(find.text(fixture.label), findsOneWidget);

      await tester.binding.handlePopRoute();
      await tester.pump();
      expect(find.text(fixture.label), findsOneWidget);
    });
  }
}

class _InviteJoinSheetLauncher extends StatelessWidget {
  const _InviteJoinSheetLauncher();

  @override
  Widget build(BuildContext context) => Scaffold(
    body: Center(
      child: FilledButton(
        onPressed: () => showInviteJoinSheet(context),
        child: const Text('Open invite'),
      ),
    ),
  );
}

class _StaticInviteJoinNotifier extends InviteJoinNotifier {
  _StaticInviteJoinNotifier(this._state);

  final InviteJoinState _state;

  @override
  InviteJoinState build() => _state;
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
