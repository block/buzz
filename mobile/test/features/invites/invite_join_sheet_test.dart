import 'package:buzz/shared/theme/theme.dart';
import 'package:buzz/features/invites/invite_join_provider.dart';
import 'package:buzz/shared/deeplink/deep_link.dart';
import 'package:buzz/features/invites/invite_join_sheet.dart';
import 'dart:async';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

import '../../helpers/widget_helpers.dart';

void main() {
  testWidgets(
    'iOS policy recovery dismisses to browser without reporting joined',
    (tester) async {
      const channel = MethodChannel('buzz/community_entry');
      final messenger =
          TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger;
      var launches = 0;
      final notifier = _PolicyInviteJoinNotifier();
      final responses = <Map>[];
      messenger.setMockMethodCallHandler(channel, (call) async {
        final config = call.arguments as Map;
        for (var attempt = 0; attempt < 2; attempt++) {
          final result = Completer<Map>();
          messenger.handlePlatformMessage(
            channel.name,
            const StandardMethodCodec().encodeMethodCall(
              MethodCall('join', {'session': config['session']}),
            ),
            (data) => result.complete(
              const StandardMethodCodec().decodeEnvelope(data!) as Map,
            ),
          );
          responses.add(await result.future);
        }
        return false;
      });
      addTearDown(() => messenger.setMockMethodCallHandler(channel, null));
      await tester.pumpWidget(
        WidgetHelpers.testable(
          child: const _InviteJoinSheetLauncher(),
          overrides: [
            inviteJoinProvider.overrideWith(() => notifier),
            invitePolicyLauncherProvider.overrideWithValue(
              (uri) async => ++launches > 1,
            ),
          ],
        ),
      );
      await tester.tap(find.text('Open invite'));
      await tester.pumpAndSettle();
      expect(notifier.claims, 1);
      expect(launches, 2);
      expect(responses.first['actionTitle'], 'Review requirements');
      expect(responses.last, {'redirected': true});
      expect(find.byType(InviteJoinSheet), findsNothing);
    },
    variant: TargetPlatformVariant.only(TargetPlatform.iOS),
  );
  testWidgets('policy rejection offers an enabled browser recovery action', (
    tester,
  ) async {
    var launches = 0;
    await tester.pumpWidget(
      WidgetHelpers.testable(
        child: const InviteJoinSheet(),
        overrides: [
          inviteJoinProvider.overrideWith(
            () => _StaticInviteJoinNotifier(
              const InviteJoinState(
                status: InviteJoinStatus.error,
                invite: InviteDeepLink(
                  relayUrl: 'wss://relay.example.com',
                  code: 'code',
                ),
                requiresFreshInvite: true,
                requiresPolicyAcceptance: true,
              ),
            ),
          ),
          invitePolicyLauncherProvider.overrideWithValue((uri) async {
            launches++;
            return false;
          }),
        ],
      ),
    );
    final button = find.widgetWithText(FilledButton, 'Review requirements');
    await tester.ensureVisible(button);
    await tester.tap(button);
    await tester.pump();
    expect(launches, 1);
    expect(find.textContaining('Could not open the browser'), findsOneWidget);
    expect(tester.widget<FilledButton>(button).onPressed, isNotNull);
  });
  testWidgets('native dark Join uses the theme foreground on its accent fill', (
    tester,
  ) async {
    const channel = MethodChannel('buzz/community_entry');
    final messenger =
        TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger;
    Map? config;
    messenger.setMockMethodCallHandler(channel, (call) async {
      config = call.arguments as Map;
      return false;
    });
    addTearDown(() => messenger.setMockMethodCallHandler(channel, null));
    final theme = AppTheme.dark();
    await tester.pumpWidget(
      WidgetHelpers.testable(
        child: Theme(data: theme, child: const _InviteJoinSheetLauncher()),
        overrides: [
          inviteJoinProvider.overrideWith(
            () => _StaticInviteJoinNotifier(
              const InviteJoinState(status: InviteJoinStatus.confirming),
            ),
          ),
        ],
      ),
    );
    await tester.tap(find.text('Open invite'));
    await tester.pumpAndSettle();
    expect(config?['primaryArgb'], theme.colorScheme.primary.toARGB32());
    expect(config?['onPrimaryArgb'], theme.colorScheme.onPrimary.toARGB32());
  }, variant: TargetPlatformVariant.only(TargetPlatform.iOS));

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

class _PolicyInviteJoinNotifier extends InviteJoinNotifier {
  var claims = 0;
  @override
  InviteJoinState build() => const InviteJoinState(
    status: InviteJoinStatus.confirming,
    invite: InviteDeepLink(relayUrl: 'wss://relay.example.com', code: 'code'),
  );
  @override
  Future<void> confirmJoin() async {
    claims++;
    state = state.copyWith(
      status: InviteJoinStatus.error,
      requiresFreshInvite: true,
      requiresPolicyAcceptance: true,
    );
  }
}
