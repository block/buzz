import 'dart:async';

import 'package:buzz/app.dart';
import 'package:buzz/shared/security/sensitive_action_authorizer.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

void main() {
  test('authorization session shares an in-flight device prompt', () async {
    final pending = Completer<DeviceAuthResult>();
    final authorizer = _FakeAuthorizer(pending: pending);
    final session = SensitiveActionAuthorizationSession(
      authorizer: authorizer,
      now: () => DateTime.utc(2026, 8, 6),
    );

    final first = session.authorize();
    final second = session.authorize();
    expect(authorizer.calls, 1);
    expect(session.isAuthorizing, isTrue);

    pending.complete(DeviceAuthResult.success);
    expect(await first, DeviceAuthResult.success);
    expect(await second, DeviceAuthResult.success);
    expect(session.isAuthorizing, isFalse);
  });

  testWidgets('protected cold launch authenticates before showing content', (
    tester,
  ) async {
    final authorizer = _FakeAuthorizer();
    await tester.pumpWidget(_testApp(authorizer: authorizer));
    await tester.pump();

    expect(authorizer.calls, 1);
    expect(find.text('Private content'), findsOneWidget);
    expect(find.byKey(const Key('app-lock-screen')), findsNothing);
  });

  testWidgets('cancelled cold launch stays on privacy-safe lock screen', (
    tester,
  ) async {
    final authorizer = _FakeAuthorizer(result: DeviceAuthResult.cancelled);
    await tester.pumpWidget(_testApp(authorizer: authorizer));
    await tester.pump();

    expect(find.byKey(const Key('app-lock-screen')), findsOneWidget);
    expect(find.byKey(const Key('app-lock-logo')), findsOneWidget);
    expect(find.text('Private content'), findsNothing);
    expect(find.text('Buzz is locked'), findsNothing);
    expect(find.text('Unlock with Face ID'), findsOneWidget);

    final scaffold = tester.widget<Scaffold>(
      find.byKey(const Key('app-lock-screen')),
    );
    expect(scaffold.backgroundColor, Colors.black);
    final button = tester.widget<FilledButton>(
      find.byKey(const Key('app-lock-unlock-button')),
    );
    expect(
      button.style?.backgroundColor?.resolve(<WidgetState>{}),
      Colors.white,
    );
    expect(
      button.style?.foregroundColor?.resolve(<WidgetState>{}),
      Colors.black,
    );
  });

  testWidgets('resume inside five minutes remains unlocked', (tester) async {
    var now = DateTime.utc(2026, 8, 6);
    final authorizer = _FakeAuthorizer();
    await tester.pumpWidget(_testApp(authorizer: authorizer, now: () => now));
    await tester.pump();

    tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.paused);

    now = now.add(const Duration(minutes: 4));
    tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.resumed);
    await tester.pump();

    expect(authorizer.calls, 1);
    expect(find.text('Private content'), findsOneWidget);
  });

  testWidgets('resume after five minutes requires fresh authentication', (
    tester,
  ) async {
    var now = DateTime.utc(2026, 8, 6);
    final authorizer = _FakeAuthorizer();
    await tester.pumpWidget(_testApp(authorizer: authorizer, now: () => now));
    await tester.pump();

    tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.paused);
    now = now.add(const Duration(minutes: 5));
    tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.resumed);
    await tester.pump();

    expect(authorizer.calls, 2);
    expect(find.text('Private content'), findsOneWidget);
  });

  testWidgets('locking preserves authenticated child state', (tester) async {
    final authorizer = _FakeAuthorizer();
    await tester.pumpWidget(
      _testApp(authorizer: authorizer, child: const _StatefulChild()),
    );
    await tester.pump();
    await tester.tap(find.text('Increment'));
    await tester.pump();
    expect(find.text('Count: 1'), findsOneWidget);

    tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.paused);
    await tester.pump();
    expect(find.byKey(const Key('app-lock-screen')), findsOneWidget);
    expect(find.text('Count: 1', skipOffstage: false), findsOneWidget);

    tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.resumed);
    await tester.pump();
    expect(find.text('Count: 1'), findsOneWidget);
  });

  testWidgets('protected community change requires fresh authentication', (
    tester,
  ) async {
    final authorizer = _FakeAuthorizer();
    await tester.pumpWidget(
      _testApp(authorizer: authorizer, communityId: 'community-a'),
    );
    await tester.pump();

    await tester.pumpWidget(
      _testApp(authorizer: authorizer, communityId: 'community-b'),
    );
    await tester.pump();

    expect(authorizer.calls, 2);
  });

  testWidgets('protected-action authorization suppresses lifecycle prompt', (
    tester,
  ) async {
    final pending = Completer<DeviceAuthResult>();
    final authorizer = _FakeAuthorizer(pending: pending);
    await tester.pumpWidget(_testApp(authorizer: authorizer));
    await tester.pump();
    expect(authorizer.calls, 1);

    tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.inactive);
    tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.resumed);
    await tester.pump();
    expect(authorizer.calls, 1);

    pending.complete(DeviceAuthResult.success);
    await tester.pump();
  });

  testWidgets('lock screen offers subtle leave community action', (
    tester,
  ) async {
    final authorizer = _FakeAuthorizer(result: DeviceAuthResult.unavailable);
    await tester.pumpWidget(_testApp(authorizer: authorizer));
    await tester.pump();

    expect(find.text('Leave community'), findsOneWidget);
    final button = tester.widget<TextButton>(
      find.byKey(const Key('app-lock-leave-community')),
    );
    expect(
      button.style?.foregroundColor?.resolve(<WidgetState>{}),
      Colors.white54,
    );
  });

  testWidgets('disabled protection never authenticates', (tester) async {
    final authorizer = _FakeAuthorizer();
    await tester.pumpWidget(_testApp(authorizer: authorizer, enabled: false));
    await tester.pump();

    expect(authorizer.calls, 0);
    expect(find.text('Private content'), findsOneWidget);
  });
}

Widget _testApp({
  required _FakeAuthorizer authorizer,
  DateTime Function()? now,
  bool enabled = true,
  String? communityId,
  Widget child = const Scaffold(body: Text('Private content')),
}) {
  return ProviderScope(
    overrides: [
      sensitiveActionAuthorizerProvider.overrideWithValue(authorizer),
      if (now != null) appLockClockProvider.overrideWithValue(now),
    ],
    child: MaterialApp(
      theme: ThemeData(platform: TargetPlatform.iOS),
      home: AppLockGate(
        enabled: enabled,
        communityId: communityId,
        child: child,
      ),
    ),
  );
}

class _StatefulChild extends StatefulWidget {
  const _StatefulChild();

  @override
  State<_StatefulChild> createState() => _StatefulChildState();
}

class _StatefulChildState extends State<_StatefulChild> {
  var count = 0;

  @override
  Widget build(BuildContext context) => Scaffold(
    body: Column(
      children: [
        Text('Count: $count'),
        TextButton(
          onPressed: () => setState(() => count++),
          child: const Text('Increment'),
        ),
      ],
    ),
  );
}

class _FakeAuthorizer implements SensitiveActionAuthorizer {
  _FakeAuthorizer({this.result = DeviceAuthResult.success, this.pending});

  DeviceAuthResult result;
  final Completer<DeviceAuthResult>? pending;
  int calls = 0;

  @override
  Future<DeviceAuthResult> authorizeIdentityAction() async {
    calls++;
    return pending?.future ?? result;
  }

  @override
  Future<bool> isSupported() async => true;
}
