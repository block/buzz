import 'dart:async';

import 'package:buzz/features/pairing/pairing_provider.dart';
import 'package:buzz/features/settings/settings_page.dart';
import 'package:buzz/shared/auth/auth.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:shared_preferences/shared_preferences.dart';

import '../../helpers/widget_helpers.dart';

void main() {
  testWidgets('shows a compact copyable identity row', (tester) async {
    MethodCall? clipboardCall;
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(SystemChannels.platform, (call) async {
          if (call.method == 'Clipboard.setData') clipboardCall = call;
          return null;
        });
    addTearDown(
      () => TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
          .setMockMethodCallHandler(SystemChannels.platform, null),
    );
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();

    await tester.pumpWidget(
      WidgetHelpers.testable(
        overrides: [
          relayConfigProvider.overrideWith(_RelayConfigNotifier.new),
          authProvider.overrideWith(_AuthNotifier.new),
          pairingProvider.overrideWith(
            () => _PairingNotifier(Future<bool>.value(true)),
          ),
          savedPrefsProvider.overrideWithValue(prefs),
        ],
        child: SettingsPage(
          profileHeader: const SizedBox.shrink(),
          invitePageBuilder: (_) => const SizedBox.shrink(),
          identityRecoveryPageBuilder: (_) => const SizedBox.shrink(),
        ),
      ),
    );
    await tester.pump();
    await tester.ensureVisible(find.text('Identity (pubkey)'));
    await tester.pumpAndSettle();

    final expectedPubkey = nostr.Keys(
      '1111111111111111111111111111111111111111111111111111111111111111',
    ).public;
    expect(find.text('Connected to'), findsNothing);
    expect(find.text('https://relay.test'), findsNothing);
    expect(find.text(expectedPubkey), findsNothing);
    final copy = tester.getRect(find.byIcon(LucideIcons.copy));
    final chevron = tester.getRect(find.byIcon(LucideIcons.chevronRight).first);
    expect(copy.center.dx, closeTo(chevron.center.dx, 0.5));

    await tester.tap(find.text('Identity (pubkey)'));
    await tester.pump();
    expect(clipboardCall?.method, 'Clipboard.setData');
    expect(clipboardCall?.arguments, {'text': expectedPubkey});
    expect(find.text('Pubkey copied'), findsOneWidget);
  });

  testWidgets('waits for a resumed frame before navigating after auth', (
    tester,
  ) async {
    final authorization = Completer<bool>();
    final pairing = _PairingNotifier(authorization.future);
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();

    await tester.pumpWidget(
      WidgetHelpers.testable(
        overrides: [
          relayConfigProvider.overrideWith(_RelayConfigNotifier.new),
          authProvider.overrideWith(_AuthNotifier.new),
          pairingProvider.overrideWith(() => pairing),
          savedPrefsProvider.overrideWithValue(prefs),
        ],
        child: SettingsPage(
          profileHeader: const SizedBox.shrink(),
          invitePageBuilder: (_) => const SizedBox.shrink(),
          identityRecoveryPageBuilder: (_) =>
              const Scaffold(body: Text('Identity recovery')),
        ),
      ),
    );
    await tester.pump();

    await tester.tap(find.text('Send identity to desktop'));
    await tester.pump();
    tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.inactive);
    authorization.complete(true);
    await tester.pump();

    expect(find.text('Identity recovery'), findsNothing);

    tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.resumed);
    await tester.pump();
    await tester.pump();
    await tester.pumpAndSettle();

    expect(find.text('Identity recovery'), findsOneWidget);
  });
  testWidgets('resume timeout keeps identity recovery closed', (tester) async {
    final pairing = _PairingNotifier(Future<bool>.value(true));
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();

    await tester.pumpWidget(
      WidgetHelpers.testable(
        overrides: [
          relayConfigProvider.overrideWith(_RelayConfigNotifier.new),
          authProvider.overrideWith(_AuthNotifier.new),
          pairingProvider.overrideWith(() => pairing),
          savedPrefsProvider.overrideWithValue(prefs),
        ],
        child: SettingsPage(
          profileHeader: const SizedBox.shrink(),
          invitePageBuilder: (_) => const SizedBox.shrink(),
          identityRecoveryPageBuilder: (_) =>
              const Scaffold(body: Text('Identity recovery')),
        ),
      ),
    );
    await tester.pump();
    tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.inactive);

    await tester.tap(find.text('Send identity to desktop'));
    await tester.pump();
    await tester.pump(const Duration(seconds: 5));
    await tester.pump();

    expect(pairing.resetCalls, 1);
    expect(find.text('Identity recovery'), findsNothing);
    expect(
      find.text('Buzz did not return to the foreground. Try again.'),
      findsOneWidget,
    );
  });

  testWidgets('clears authorization when disposed during authentication', (
    tester,
  ) async {
    final authorization = Completer<bool>();
    final pairing = _PairingNotifier(authorization.future);
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();

    await tester.pumpWidget(
      WidgetHelpers.testable(
        overrides: [
          relayConfigProvider.overrideWith(_RelayConfigNotifier.new),
          authProvider.overrideWith(_AuthNotifier.new),
          pairingProvider.overrideWith(() => pairing),
          savedPrefsProvider.overrideWithValue(prefs),
        ],
        child: SettingsPage(
          profileHeader: const SizedBox.shrink(),
          invitePageBuilder: (_) => const SizedBox.shrink(),
          identityRecoveryPageBuilder: (_) =>
              const Scaffold(body: Text('Identity recovery')),
        ),
      ),
    );
    await tester.pump();

    final settingsContext = tester.element(
      find.text('Send identity to desktop'),
    );
    await tester.tap(find.text('Send identity to desktop'));
    await tester.pump();
    unawaited(
      Navigator.of(settingsContext).pushReplacement(
        MaterialPageRoute<void>(builder: (_) => const SizedBox.shrink()),
      ),
    );
    await tester.pumpAndSettle();
    authorization.complete(true);
    await tester.pump();

    expect(pairing.resetCalls, 1);
    expect(find.text('Identity recovery'), findsNothing);
  });

  testWidgets('clears authorization when disposed during resume wait', (
    tester,
  ) async {
    final pairing = _PairingNotifier(Future<bool>.value(true));
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();

    await tester.pumpWidget(
      WidgetHelpers.testable(
        overrides: [
          relayConfigProvider.overrideWith(_RelayConfigNotifier.new),
          authProvider.overrideWith(_AuthNotifier.new),
          pairingProvider.overrideWith(() => pairing),
          savedPrefsProvider.overrideWithValue(prefs),
        ],
        child: SettingsPage(
          profileHeader: const SizedBox.shrink(),
          invitePageBuilder: (_) => const SizedBox.shrink(),
          identityRecoveryPageBuilder: (_) =>
              const Scaffold(body: Text('Identity recovery')),
        ),
      ),
    );
    await tester.pump();
    tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.inactive);

    final settingsContext = tester.element(
      find.text('Send identity to desktop'),
    );
    await tester.tap(find.text('Send identity to desktop'));
    await tester.pump();
    unawaited(
      Navigator.of(settingsContext).pushReplacement(
        MaterialPageRoute<void>(builder: (_) => const SizedBox.shrink()),
      ),
    );
    await tester.pumpAndSettle();
    await tester.pump(const Duration(seconds: 5));
    await tester.pump();

    expect(pairing.resetCalls, 1);
    expect(find.text('Identity recovery'), findsNothing);
  });

  testWidgets('denied authentication does not open identity recovery', (
    tester,
  ) async {
    final pairing = _PairingNotifier(Future<bool>.value(false));
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();

    await tester.pumpWidget(
      WidgetHelpers.testable(
        overrides: [
          relayConfigProvider.overrideWith(_RelayConfigNotifier.new),
          authProvider.overrideWith(_AuthNotifier.new),
          pairingProvider.overrideWith(() => pairing),
          savedPrefsProvider.overrideWithValue(prefs),
        ],
        child: SettingsPage(
          profileHeader: const SizedBox.shrink(),
          invitePageBuilder: (_) => const SizedBox.shrink(),
          identityRecoveryPageBuilder: (_) =>
              const Scaffold(body: Text('Identity recovery')),
        ),
      ),
    );
    await tester.pump();

    await tester.tap(find.text('Send identity to desktop'));
    await tester.pumpAndSettle();

    expect(pairing.authorizationCalls, 1);
    expect(find.text('Identity recovery'), findsNothing);
  });
}

class _AuthNotifier extends AuthNotifier {
  @override
  Future<AuthState> build() async => AuthState(
    status: AuthStatus.authenticated,
    community: Community(
      id: 'community',
      name: 'Test',
      relayUrl: 'https://relay.test',
      nsec: _RelayConfigNotifier.nsec,
      addedAt: DateTime.utc(2026),
    ),
  );
}

class _RelayConfigNotifier extends RelayConfigNotifier {
  static final nsec = nostr.Keys(
    '1111111111111111111111111111111111111111111111111111111111111111',
  ).nsec;

  @override
  RelayConfig build() => RelayConfig(baseUrl: 'https://relay.test', nsec: nsec);
}

class _PairingNotifier extends PairingNotifier {
  _PairingNotifier(this.authorization);

  final Future<bool> authorization;
  int authorizationCalls = 0;
  int resetCalls = 0;

  @override
  PairingState build() => const PairingState();

  @override
  Future<bool> authorizeIdentityExport({required Community community}) {
    authorizationCalls++;
    return authorization;
  }

  @override
  void reset() {
    resetCalls++;
  }
}
