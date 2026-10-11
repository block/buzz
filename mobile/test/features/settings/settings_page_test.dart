import 'dart:async';
import 'dart:io';
import 'dart:ui' as ui;
import 'package:flutter/rendering.dart';
import 'package:flutter/services.dart';

import 'package:buzz/features/settings/settings_page.dart';
import 'package:buzz/shared/community/community_membership_provider.dart';
import 'package:buzz/shared/community/community.dart';
import 'package:buzz/shared/community/community_provider.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:buzz/shared/push/push_bridge.dart';
import 'package:buzz/shared/push/dev_push_lease.dart';
import 'package:buzz/shared/push/push_relay_capability_provider.dart';
import 'package:buzz/shared/relay/app_lifecycle_provider.dart';
import 'package:buzz/shared/widgets/app_list.dart';
import 'package:buzz/shared/widgets/app_list_card.dart';
import 'package:flutter/material.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:package_info_plus/package_info_plus.dart';
import 'package:shared_preferences/shared_preferences.dart';

void main() {
  setUp(() {
    PackageInfo.setMockInitialValues(
      appName: 'Buzz',
      packageName: 'xyz.block.buzz',
      version: '0.16.0',
      buildNumber: '432',
      buildSignature: '',
    );
  });

  for (final buildNumber in ['432', '', '2147483647']) {
    testWidgets('shows version with build number "$buildNumber"', (
      tester,
    ) async {
      tester.view.physicalSize = const Size(320, 700);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.resetPhysicalSize);
      addTearDown(tester.view.resetDevicePixelRatio);
      PackageInfo.setMockInitialValues(
        appName: 'Buzz',
        packageName: 'xyz.block.buzz',
        version: '0.16.0',
        buildNumber: buildNumber,
        buildSignature: '',
      );
      SharedPreferences.setMockInitialValues({});
      final prefs = await SharedPreferences.getInstance();
      await tester.pumpWidget(
        ProviderScope(
          overrides: [
            savedPrefsProvider.overrideWithValue(prefs),
            currentCommunityRoleProvider.overrideWithValue(
              const AsyncData<CommunityMemberRole?>(CommunityMemberRole.admin),
            ),
          ],
          child: MaterialApp(
            theme: AppTheme.light(),
            builder: (context, child) => MediaQuery(
              data: MediaQuery.of(
                context,
              ).copyWith(textScaler: TextScaler.linear(2)),
              child: child!,
            ),
            home: SettingsPage(
              profileHeader: const SizedBox.shrink(),
              identityRecoveryPageBuilder: (_) => const SizedBox.shrink(),
            ),
          ),
        ),
      );
      expect(find.text('Invite to community'), findsNothing);
      await tester.pumpAndSettle();
      final version = find.text(
        buildNumber.isEmpty ? 'v0.16.0' : 'v0.16.0 ($buildNumber)',
      );
      await tester.scrollUntilVisible(version, 200);
      await tester.pumpAndSettle();
      expect(version.hitTestable(), findsOneWidget);
      expect(
        find.ancestor(of: version, matching: find.byType(ListView)),
        findsOneWidget,
      );
      final beforeScroll = tester.getTopLeft(version).dy;
      await tester.drag(find.byType(ListView), const Offset(0, 100));
      await tester.pumpAndSettle();
      expect(tester.getTopLeft(version).dy, greaterThan(beforeScroll));
      expect(tester.takeException(), isNull);
    });
  }

  for (final outcome in ['absent', 'error']) {
    testWidgets('hides notifications while capability loads and is $outcome', (
      tester,
    ) async {
      debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
      addTearDown(() => debugDefaultTargetPlatformOverride = null);
      SharedPreferences.setMockInitialValues({});
      final prefs = await SharedPreferences.getInstance();
      final capability = Completer<BuzzPushLeaseDescriptor?>();
      var permissionReads = 0;
      await tester.pumpWidget(
        ProviderScope(
          overrides: [
            savedPrefsProvider.overrideWithValue(prefs),
            activeCommunityProvider.overrideWith(
              (ref) async => Community.create(
                name: 'No push',
                relayUrl: 'wss://relay.example',
              ).copyWith(pushNotificationsEnabled: false),
            ),
            currentRelayPushDescriptorProvider.overrideWith(
              (ref) => capability.future,
            ),
            buzzPushAuthorizationStatusReaderProvider.overrideWithValue(
              () async {
                permissionReads += 1;
                return BuzzPushAuthorizationStatus.authorized;
              },
            ),
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
      expect(find.text('Notifications'), findsNothing);
      expect(find.byType(Switch), findsNothing);
      if (outcome == 'absent') {
        capability.complete(null);
      } else {
        capability.completeError(StateError('discovery failed'));
      }
      await tester.pumpAndSettle();
      expect(find.text('Notifications'), findsNothing);
      expect(find.text('Push notifications'), findsNothing);
      expect(find.byType(Switch), findsNothing);
      expect(permissionReads, 0);
      expect(tester.takeException(), isNull);
      debugDefaultTargetPlatformOverride = null;
    });
  }

  testWidgets('shows the persisted per-community push opt-in on iOS', (
    tester,
  ) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
    addTearDown(() => debugDefaultTargetPlatformOverride = null);
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();
    final community = Community.create(
      name: 'Team',
      relayUrl: 'wss://relay.example',
    ).copyWith(pushNotificationsEnabled: true);

    final capability = Future<BuzzPushLeaseDescriptor?>.value(_pushDescriptor);

    await tester.pumpWidget(
      ProviderScope(
        overrides: [
          savedPrefsProvider.overrideWithValue(prefs),
          activeCommunityProvider.overrideWith((ref) async => community),
          currentRelayPushDescriptorProvider.overrideWith((ref) => capability),
          appLifecycleProvider.overrideWith(_SettingsLifecycleNotifier.new),
          buzzPushAuthorizationStatusReaderProvider.overrideWithValue(
            () async => BuzzPushAuthorizationStatus.authorized,
          ),
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

    expect(
      find.byKey(const ValueKey('push-notifications-enabled')),
      findsOneWidget,
    );
    expect(tester.widget<Switch>(find.byType(Switch)).value, isTrue);
    debugDefaultTargetPlatformOverride = null;
  });

  for (final brightness in Brightness.values) {
    testWidgets(
      'shows status above profile editing and routes photo directly in ${brightness.name}',
      (tester) async {
        if (Platform.environment.containsKey('PROFILE_SCREENSHOTS')) {
          await tester.runAsync(() async {
            for (final font in {
              'Inter': 'assets/fonts/InterVariable.ttf',
              'BuzzTabler': 'assets/fonts/TablerIcons.ttf',
            }.entries) {
              await (FontLoader(
                font.key,
              )..addFont(rootBundle.load(font.value))).load();
            }
          });
        }
        tester.view.physicalSize = const Size(390, 844);
        tester.view.devicePixelRatio = 1;
        addTearDown(tester.view.reset);
        SharedPreferences.setMockInitialValues({});
        final prefs = await SharedPreferences.getInstance();

        await tester.pumpWidget(
          ProviderScope(
            overrides: [savedPrefsProvider.overrideWithValue(prefs)],
            child: MaterialApp(
              theme: brightness == Brightness.dark
                  ? AppTheme.dark()
                  : AppTheme.light(),
              home: SettingsPage(
                profileHeader: const SizedBox.square(dimension: 128),
                profileEditPageBuilder: (_) =>
                    const Scaffold(body: Text('Profile editor destination')),
                identityRecoveryPageBuilder: (_) => const SizedBox.shrink(),
              ),
            ),
          ),
        );
        await tester.pumpAndSettle();

        expect(find.text('Profile'), findsNothing);
        expect(find.text('Edit profile'), findsNothing);
        expect(find.text('Display name'), findsOneWidget);
        expect(find.text('Profile description'), findsOneWidget);
        expect(find.text('Edit photo'), findsOneWidget);
        final optionCard = find.descendant(
          of: find.byKey(const ValueKey('edit-profile-options')),
          matching: find.byType(Material),
        );
        expect(tester.getSize(optionCard.first).height, greaterThan(150));
        for (final key in const [
          'edit-profile-display-name',
          'edit-profile-description',
          'edit-profile-photo',
        ]) {
          expect(
            tester
                .widget<AppListRow>(find.byKey(ValueKey(key)))
                .verticalPadding,
            Grid.xs,
          );
        }
        expect(
          find.byKey(const ValueKey('settings-edit-profile')),
          findsNothing,
        );
        expect(find.byType(BottomSheet), findsNothing);
        expect(
          tester.widget<AppListCard>(find.byType(AppListCard).first).key,
          const ValueKey('status-identity-options'),
        );
        if (Platform.environment['PROFILE_SCREENSHOTS'] case final directory?) {
          final boundary = tester
              .element(find.byType(SettingsPage))
              .findAncestorRenderObjectOfType<RenderRepaintBoundary>()!;
          await tester.runAsync(() async {
            final image = await boundary.toImage(pixelRatio: 2);
            final bytes = await image.toByteData(
              format: ui.ImageByteFormat.png,
            );
            await Directory(directory).create(recursive: true);
            await File(
              '$directory/profile-${brightness.name}.png',
            ).writeAsBytes(bytes!.buffer.asUint8List());
            image.dispose();
          });
        }
        await tester.tap(find.byKey(const ValueKey('edit-profile-photo')));
        await tester.pumpAndSettle();
        expect(find.text('Profile editor destination'), findsOneWidget);
      },
    );
  }

  testWidgets('opens profile text editors directly from settings', (
    tester,
  ) async {
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();
    final opened = <String>[];

    await tester.pumpWidget(
      ProviderScope(
        overrides: [savedPrefsProvider.overrideWithValue(prefs)],
        child: MaterialApp(
          theme: AppTheme.light(),
          home: SettingsPage(
            profileHeader: const SizedBox.shrink(),
            onSetStatus: (_) => opened.add('status'),
            onEditDisplayName: (_) async => opened.add('name'),
            onEditProfileDescription: (_) async => opened.add('description'),
            identityRecoveryPageBuilder: (_) => const SizedBox.shrink(),
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();

    expect(
      tester.getTopLeft(find.text('Set status')).dy,
      lessThan(tester.getTopLeft(find.text('Display name')).dy),
    );
    await tester.tap(find.text('Set status'));
    expect(opened, ['status']);
    await tester.tap(find.byKey(const ValueKey('edit-profile-display-name')));
    await tester.pumpAndSettle();
    expect(opened, ['status', 'name']);
    expect(find.text('Edit profile'), findsNothing);

    await tester.tap(find.byKey(const ValueKey('edit-profile-description')));
    await tester.pumpAndSettle();
    expect(opened, ['status', 'name', 'description']);
  });

  testWidgets('uses native navigation with Close and no Edit action on iOS', (
    tester,
  ) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
    addTearDown(() => debugDefaultTargetPlatformOverride = null);
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();

    await tester.pumpWidget(
      ProviderScope(
        overrides: [savedPrefsProvider.overrideWithValue(prefs)],
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

    final nativeBar = tester.widget<UiKitView>(find.byType(UiKitView));
    expect(nativeBar.viewType, 'buzz/ios_navigation_bar');
    final params = nativeBar.creationParams! as Map<String, Object?>;
    expect(params['title'], 'Settings');
    expect(params['largeTitle'], isFalse);
    expect(params['leading'], containsPair('symbol', 'xmark'));
    expect(params['leading'], containsPair('label', 'Close settings'));
    expect(params['actions'], isEmpty);

    debugDefaultTargetPlatformOverride = null;
  });

  testWidgets('keeps community controls out of personal settings', (
    tester,
  ) async {
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();

    await tester.pumpWidget(
      ProviderScope(
        overrides: [
          savedPrefsProvider.overrideWithValue(prefs),
          currentCommunityRoleProvider.overrideWithValue(
            const AsyncData<CommunityMemberRole?>(CommunityMemberRole.admin),
          ),
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

    expect(find.text('Invite to community'), findsNothing);
    expect(
      find.text('Add people directly or share an invite link'),
      findsNothing,
    );
  });

  testWidgets('keeps personal settings independent of community role errors', (
    tester,
  ) async {
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();

    await tester.pumpWidget(
      ProviderScope(
        overrides: [
          savedPrefsProvider.overrideWithValue(prefs),
          currentCommunityRoleProvider.overrideWithValue(
            AsyncError<CommunityMemberRole?>(
              Exception('membership query failed'),
              StackTrace.empty,
            ),
          ),
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

    expect(find.text('Invite to community'), findsNothing);
  });

  testWidgets('hides community invite navigation from plain members', (
    tester,
  ) async {
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();

    await tester.pumpWidget(
      ProviderScope(
        overrides: [
          savedPrefsProvider.overrideWithValue(prefs),
          currentCommunityRoleProvider.overrideWithValue(
            const AsyncData<CommunityMemberRole?>(CommunityMemberRole.member),
          ),
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

    expect(find.text('Invite to community'), findsNothing);
  });

  testWidgets('uses a 24dp rhythm between profile settings groups', (
    tester,
  ) async {
    tester.view.physicalSize = const Size(390, 1600);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.reset);
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();

    await tester.pumpWidget(
      ProviderScope(
        overrides: [
          savedPrefsProvider.overrideWithValue(prefs),
          currentCommunityRoleProvider.overrideWithValue(
            const AsyncData<CommunityMemberRole?>(CommunityMemberRole.admin),
          ),
        ],
        child: MaterialApp(
          theme: AppTheme.light(),
          home: SettingsPage(
            profileHeader: const SizedBox(height: 100),
            identityRecoveryPageBuilder: (_) => const SizedBox.shrink(),
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();

    final sections = tester.widgetList<AppListCard>(find.byType(AppListCard));
    expect(sections, isNotEmpty);
    expect(
      sections.every((section) => section.verticalPadding == Grid.twelve),
      isTrue,
    );
  });
}

class _SettingsLifecycleNotifier extends AppLifecycleNotifier {
  @override
  AppLifecycleState build() => AppLifecycleState.resumed;
}

const _pushDescriptor = BuzzPushLeaseDescriptor(
  origin: 'wss://relay.example',
  executorKeyId: 'key',
  executorPubkey: 'pubkey',
  transport: 'apns',
  maxLeaseTtlSeconds: 3600,
  maxContentLength: 4096,
  maxPlaintextLength: 4096,
  maxEndpointLength: 2048,
  maxStringLength: 512,
);
