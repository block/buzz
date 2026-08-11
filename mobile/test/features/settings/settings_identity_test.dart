import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:shared_preferences/shared_preferences.dart';
import 'package:buzz/features/settings/settings_page.dart';
import 'package:buzz/shared/auth/auth.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/theme.dart';

import '../../helpers/widget_helpers.dart';

void main() {
  testWidgets('connection identity displays and copies full npub', (
    tester,
  ) async {
    final keys = nostr.Keys(
      '1111111111111111111111111111111111111111111111111111111111111111',
    );
    SharedPreferences.setMockInitialValues(const {});
    final prefs = await SharedPreferences.getInstance();
    String? copiedText;
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(SystemChannels.platform, (call) async {
          if (call.method == 'Clipboard.setData') {
            copiedText =
                (call.arguments as Map<Object?, Object?>)['text'] as String?;
          }
          return null;
        });
    addTearDown(
      () => TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
          .setMockMethodCallHandler(SystemChannels.platform, null),
    );

    await tester.pumpWidget(
      WidgetHelpers.testable(
        child: SettingsPage(
          profileHeader: const SizedBox.shrink(),
          invitePageBuilder: (_) => const SizedBox.shrink(),
          identityRecoveryPageBuilder: (_) => const SizedBox.shrink(),
        ),
        overrides: [
          savedPrefsProvider.overrideWithValue(prefs),
          authProvider.overrideWith(() => _IdentityAuth(keys.nsec)),
          relayConfigProvider.overrideWith(
            () => _IdentityRelayConfig(keys.nsec),
          ),
        ],
      ),
    );
    await tester.pumpAndSettle();

    expect(find.text('Identity (npub)'), findsOneWidget);
    expect(find.text(keys.npub), findsOneWidget);
    expect(find.text(keys.public), findsNothing);

    await tester.ensureVisible(find.byTooltip('Copy npub'));
    await tester.tap(find.byTooltip('Copy npub'));
    await tester.pump();

    expect(copiedText, keys.npub);
    expect(find.text('npub copied'), findsOneWidget);
  });
}

class _IdentityAuth extends AuthNotifier {
  final String nsec;

  _IdentityAuth(this.nsec);

  @override
  Future<AuthState> build() async => AuthState(
    status: AuthStatus.authenticated,
    community: Community(
      id: 'community',
      name: 'Test',
      relayUrl: 'https://relay.example.com',
      nsec: nsec,
      addedAt: DateTime.utc(2026),
    ),
  );
}

class _IdentityRelayConfig extends RelayConfigNotifier {
  final String nsec;

  _IdentityRelayConfig(this.nsec);

  @override
  RelayConfig build() =>
      RelayConfig(baseUrl: 'https://relay.example.com', nsec: nsec);
}
