import 'package:buzz/features/settings/settings_page.dart';
import 'package:buzz/shared/community/community_membership_provider.dart';
import 'package:buzz/shared/hosted/hosted_environment.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:package_info_plus/package_info_plus.dart';
import 'package:shared_preferences/shared_preferences.dart';

void main() {
  Future<List<Uri>> pumpSettings(
    WidgetTester tester, {
    bool wide = false,
    bool opens = true,
    bool throws = false,
  }) async {
    tester.view.devicePixelRatio = 1;
    tester.view.physicalSize = wide
        ? const Size(1000, 800)
        : const Size(390, 844);
    addTearDown(tester.view.reset);
    PackageInfo.setMockInitialValues(
      appName: 'Buzz',
      packageName: 'xyz.block.buzz',
      version: '0.16.0',
      buildNumber: '432',
      buildSignature: '',
    );
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();
    final opened = <Uri>[];
    await tester.pumpWidget(
      ProviderScope(
        overrides: [
          savedPrefsProvider.overrideWithValue(prefs),
          currentCommunityRoleProvider.overrideWithValue(
            const AsyncData<CommunityMemberRole?>(CommunityMemberRole.member),
          ),
          builderlabWebLauncherProvider.overrideWithValue((uri) async {
            opened.add(uri);
            if (throws) throw PlatformException(code: 'ACTIVITY_NOT_FOUND');
            return opens;
          }),
        ],
        child: MaterialApp(
          theme: AppTheme.light(),
          home: SettingsPage(
            profileHeader: const SizedBox.shrink(),
            identityRecoveryPageBuilder: (_) => const SizedBox.shrink(),
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();
    return opened;
  }

  final row = find.byKey(const Key('settings-builderlab-account'));

  for (final wide in [false, true]) {
    Future<void> openAccount(WidgetTester tester) async {
      if (wide) {
        await tester.tap(find.text('Account'));
        await tester.pumpAndSettle();
      }
      await tester.ensureVisible(row);
      await tester.tap(row);
      await tester.pumpAndSettle();
    }

    testWidgets('opens the public dashboard (wide=$wide)', (tester) async {
      final opened = await pumpSettings(tester, wide: wide);
      await openAccount(tester);
      expect(opened, [HostedEnvironment.origin]);
      expect(find.text('Couldn’t open Builderlab.'), findsNothing);
    }, skip: !HostedEnvironment.enabled);

    for (final throws in [false, true]) {
      testWidgets('reports browser failure (throws=$throws, wide=$wide)', (
        tester,
      ) async {
        await pumpSettings(tester, wide: wide, opens: false, throws: throws);
        await openAccount(tester);
        expect(find.text('Couldn’t open Builderlab.'), findsOneWidget);
      }, skip: !HostedEnvironment.enabled);
    }
  }

  testWidgets('is absent outside a public build', (tester) async {
    await pumpSettings(tester);
    expect(row, findsNothing);
  }, skip: HostedEnvironment.enabled);
}
