import 'package:buzz/features/settings/settings_page.dart';
import 'package:buzz/shared/community/community_membership_provider.dart';
import 'package:buzz/shared/hosted/hosted_environment.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:package_info_plus/package_info_plus.dart';
import 'package:shared_preferences/shared_preferences.dart';

void main() {
  Future<List<Uri>> pumpSettings(
    WidgetTester tester, {
    bool opens = true,
  }) async {
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

  testWidgets('opens the public Builderlab dashboard in the browser', (
    tester,
  ) async {
    final opened = await pumpSettings(tester);
    await tester.scrollUntilVisible(row, 200);
    await tester.tap(row);
    await tester.pumpAndSettle();
    expect(opened, [HostedEnvironment.origin]);
    expect(find.text('Couldn’t open Builderlab.'), findsNothing);
  }, skip: !HostedEnvironment.enabled);

  testWidgets('says so when the browser does not open', (tester) async {
    await pumpSettings(tester, opens: false);
    await tester.scrollUntilVisible(row, 200);
    await tester.tap(row);
    await tester.pumpAndSettle();
    expect(find.text('Couldn’t open Builderlab.'), findsOneWidget);
  }, skip: !HostedEnvironment.enabled);

  testWidgets('is absent outside a public build', (tester) async {
    await pumpSettings(tester);
    expect(row, findsNothing);
  }, skip: HostedEnvironment.enabled);
}
