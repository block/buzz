import 'dart:async';
import 'package:flutter/services.dart';
import 'package:buzz/features/invites/invite_join_provider.dart';
import 'package:buzz/features/invites/invite_join_sheet.dart';
import 'package:flutter/material.dart';
import 'package:buzz/shared/hosted/hosted_join_policy.dart';
import 'package:flutter_test/flutter_test.dart';

import '../../helpers/widget_helpers.dart';

void main() {
  testWidgets(
    'iOS confirmation hands a newly required policy back to Flutter',
    (tester) async {
      const channel = MethodChannel('buzz/community_entry');
      final messenger =
          TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger;
      final dismissed = Completer<bool>();
      Map<dynamic, dynamic>? presentation;
      messenger.setMockMethodCallHandler(channel, (call) async {
        presentation = call.arguments as Map;
        return dismissed.future;
      });
      addTearDown(() => messenger.setMockMethodCallHandler(channel, null));
      await tester.pumpWidget(
        WidgetHelpers.testable(
          child: const _InviteJoinSheetLauncher(),
          overrides: [
            inviteJoinProvider.overrideWith(_NewPolicyInviteJoinNotifier.new),
          ],
        ),
      );
      await tester.tap(find.text('Open invite'));
      await tester.pumpAndSettle();
      expect(presentation, isNotNull);
      final response = Completer<Map<dynamic, dynamic>>();
      messenger.handlePlatformMessage(
        channel.name,
        const StandardMethodCodec().encodeMethodCall(
          MethodCall('join', {'session': presentation!['session']}),
        ),
        (data) => response.complete(
          const StandardMethodCodec().decodeEnvelope(data!) as Map,
        ),
      );
      await tester.pump();
      expect(await response.future, {'continueInFlutter': true});
      dismissed.complete(false);
      await tester.pumpAndSettle();
      expect(find.text('Accept and join'), findsOneWidget);
      expect(find.text('Terms of service'), findsOneWidget);
      expect(find.text('Cancel'), findsOneWidget);
    },
    variant: TargetPlatformVariant.only(TargetPlatform.iOS),
  );

  testWidgets('exhausted policy invite has no enabled acceptance action', (
    tester,
  ) async {
    await tester.pumpWidget(
      WidgetHelpers.testable(
        child: const InviteJoinSheet(),
        overrides: [
          inviteJoinProvider.overrideWith(
            () => _StaticInviteJoinNotifier(
              const InviteJoinState(
                status: InviteJoinStatus.error,
                host: 'relay.example.com',
                requiresFreshInvite: true,
                errorMessage: 'Ask for a new invite.',
                joinPolicy: HostedJoinPolicy(
                  version: 'v1',
                  ageRequired: false,
                  terms: 'Terms',
                ),
              ),
            ),
          ),
        ],
      ),
    );
    expect(find.text('Accept and join'), findsNothing);
    expect(find.text('Ask for a new invite.'), findsOneWidget);
    expect(find.text('Cancel'), findsOneWidget);
  });

  testWidgets(
    'policy consent requires age confirmation and exposes cancellation',
    (tester) async {
      await tester.pumpWidget(
        WidgetHelpers.testable(
          child: const InviteJoinSheet(),
          overrides: [
            inviteJoinProvider.overrideWith(
              () => _StaticInviteJoinNotifier(
                const InviteJoinState(
                  status: InviteJoinStatus.confirming,
                  host: 'relay.example.com',
                  joinPolicy: HostedJoinPolicy(
                    version: 'v1',
                    ageRequired: true,
                    terms: 'Terms',
                  ),
                ),
              ),
            ),
          ],
        ),
      );
      final accept = find.widgetWithText(FilledButton, 'Accept and join');
      expect(tester.widget<FilledButton>(accept).onPressed, isNull);
      expect(find.text('Cancel'), findsOneWidget);
      expect(find.widgetWithText(FilledButton, 'Join'), findsNothing);
      await tester.tap(find.byType(CheckboxListTile));
      await tester.pump();
      expect(tester.widget<FilledButton>(accept).onPressed, isNotNull);
    },
  );

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

class _NewPolicyInviteJoinNotifier extends InviteJoinNotifier {
  @override
  InviteJoinState build() => const InviteJoinState(
    status: InviteJoinStatus.confirming,
    host: 'relay.example.com',
  );

  @override
  Future<void> confirmJoin({String? acceptedPolicyVersion}) async {
    state = state.copyWith(
      status: InviteJoinStatus.error,
      joinPolicy: const HostedJoinPolicy(
        version: 'new',
        ageRequired: false,
        terms: 'Terms',
      ),
    );
  }
}
