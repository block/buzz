import 'dart:io';
import 'dart:ui' as ui;

import 'package:flutter/rendering.dart';
import 'package:buzz/features/home/home_page.dart';
import 'package:buzz/features/profile/profile_avatar.dart';
import 'package:buzz/features/profile/profile_provider.dart';
import 'package:buzz/features/profile/settings_profile_header.dart';
import 'package:buzz/features/profile/user_status_provider.dart';
import 'package:buzz/features/settings/settings_page.dart';
import 'package:buzz/shared/custom_emoji/custom_emoji_provider.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/profile/user_profile.dart';
import 'package:buzz/features/profile/user_status.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:buzz/shared/widgets/frosted_app_bar.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:package_info_plus/package_info_plus.dart';
import 'package:shared_preferences/shared_preferences.dart';

void main() {
  for (final platform in [TargetPlatform.android, TargetPlatform.iOS]) {
    testWidgets('Settings overlays are visible above Home on ${platform.name}', (
      tester,
    ) async {
      if (Platform.environment.containsKey('SETTINGS_SCREENSHOTS')) {
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
      debugDefaultTargetPlatformOverride = platform;
      addTearDown(() => debugDefaultTargetPlatformOverride = null);
      tester.view.physicalSize = const Size(390, 844);
      tester.view.devicePixelRatio = 1;
      tester.view.padding = FakeViewPadding(top: 47, bottom: 34);
      tester.view.viewPadding = FakeViewPadding(top: 47, bottom: 34);
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
      final presence = _Presence();
      final nativeChannels = <MethodChannel>[];
      tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
        SystemChannels.platform_views,
        (call) async {
          if (call.method == 'create') {
            final args = call.arguments as Map;
            final channel = MethodChannel('${args['viewType']}/${args['id']}');
            nativeChannels.add(channel);
            tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
              channel,
              (_) async => null,
            );
          }
          return null;
        },
      );
      addTearDown(() {
        for (final channel in [
          SystemChannels.platform_views,
          ...nativeChannels,
        ]) {
          tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
            channel,
            null,
          );
        }
      });
      String? copied;
      tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
        SystemChannels.platform,
        (call) async {
          if (call.method == 'Clipboard.setData') {
            copied = (call.arguments as Map)['text'] as String;
          }
          return null;
        },
      );
      addTearDown(() {
        tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
          SystemChannels.platform,
          null,
        );
      });
      await tester.pumpWidget(
        ProviderScope(
          overrides: [
            savedPrefsProvider.overrideWithValue(prefs),
            relayConfigProvider.overrideWith(_Config.new),
            appLifecycleProvider.overrideWith(_Lifecycle.new),
            profileProvider.overrideWith(_Profile.new),
            presenceProvider.overrideWith(() => presence),
            userStatusProvider.overrideWith(_Status.new),
            customEmojiListProvider.overrideWithValue(const []),
          ],
          child: MaterialApp(
            theme: AppTheme.light(),
            builder: (_, child) => RepaintBoundary(
              key: const ValueKey('settings-test-capture'),
              child: child!,
            ),
            home: HomePage(
              hasUnreadInbox: false,
              settingsPageBuilder: (_) => SettingsPage(
                profileHeader: const SettingsProfileHeader(),
                identityRecoveryPageBuilder: (_) => const SizedBox.shrink(),
              ),
            ),
          ),
        ),
      );
      await tester.pumpAndSettle();
      if (platform == TargetPlatform.iOS) {
        // Invoke the same Flutter callback used by the native bar item.
        // Native actions are owned by FrostedAppBar and are independently tested.
        // Use that production callback without synthesizing a platform view tap.
        final appBar = tester
            .widgetList<FrostedAppBar>(find.byType(FrostedAppBar))
            .first;
        appBar.nativeActions!.single.onPressed!();
      } else {
        await tester.tap(find.byType(ProfileAvatar));
      }
      await tester.pumpAndSettle();
      if (platform == TargetPlatform.iOS) {
        expect(find.byType(HomePage), findsNothing);
        expect(find.byType(HomePage, skipOffstage: false), findsOneWidget);
        final menu = tester.widget<UiKitView>(
          find.byKey(const ValueKey('settings-presence-menu')),
        );
        expect(menu.viewType, 'buzz/presence_menu');
        expect(menu.creationParams, containsPair('presence', 'offline'));
        final nativeMenu = nativeChannels.singleWhere(
          (channel) => channel.name.startsWith('buzz/presence_menu/'),
        );
        await tester.binding.defaultBinaryMessenger.handlePlatformMessage(
          nativeMenu.name,
          const StandardMethodCodec().encodeMethodCall(
            const MethodCall('selected', 'online'),
          ),
          (_) {},
        );
        await tester.pumpAndSettle();
      } else {
        await tester.tap(find.byKey(const ValueKey('settings-presence-menu')));
        await tester.pumpAndSettle();
        final online = find.byKey(const ValueKey('settings-presence-online'));
        expect(online.hitTestable(), findsOneWidget);
        final fade = tester.widget<FadeTransition>(
          find.byKey(const ValueKey('activity-popover-fade')),
        );
        expect(fade.opacity.value, 1);
        final menu = tester.getRect(
          find.byKey(const ValueKey('settings-presence-popover')),
        );
        final anchor = tester.getRect(
          find.byKey(const ValueKey('settings-presence-target')),
        );
        expect(menu.center.dx, closeTo(anchor.center.dx, 0.1));
        expect(menu.top, closeTo(anchor.bottom + Grid.half, 0.1));
        expect(menu.bottom, lessThanOrEqualTo(810));
        await _capture(tester, '${platform.name}-availability');
        await tester.tapAt(const Offset(20, 700));
        await tester.pumpAndSettle();
        expect(
          find.byKey(const ValueKey('settings-presence-popover')),
          findsNothing,
        );
        expect(presence.selected, isEmpty);
        await tester.tap(find.byKey(const ValueKey('settings-presence-menu')));
        await tester.pumpAndSettle();
        await tester.tap(online);
        await tester.pumpAndSettle();
      }
      expect(presence.selected, ['online']);
      await tester.tap(find.text('Copy public key (npub)'));
      await tester.pumpAndSettle();
      expect(copied, startsWith('npub1'));
      final snackbar = find.byType(SnackBar);
      expect(snackbar, findsOneWidget);
      final settings = find.byType(SettingsPage);
      expect(find.descendant(of: settings, matching: snackbar), findsOneWidget);
      final message = find.text('Public key (npub) copied');
      expect(message.hitTestable(), findsOneWidget);
      final toastSurface = find
          .ancestor(of: message, matching: find.byType(Material))
          .first;
      expect(tester.getRect(toastSurface).bottom, lessThanOrEqualTo(810));
      await _capture(tester, '${platform.name}-copied');
      Navigator.of(tester.element(settings)).pop();
      await tester.pumpAndSettle();
      expect(find.byType(SnackBar), findsNothing);
      expect(tester.takeException(), isNull);
      debugDefaultTargetPlatformOverride = null;
    });
  }
}

class _Presence extends PresenceNotifier {
  final selected = <String>[];
  @override
  Future<String> build() async => 'offline';
  @override
  Future<void> setPresence(String status) async {
    selected.add(status);
    state = AsyncData(status);
  }
}

class _Profile extends ProfileNotifier {
  @override
  Future<UserProfile?> build() async =>
      UserProfile(pubkey: 'aabb', displayName: 'Test');
}

class _Status extends UserStatusNotifier {
  @override
  Future<UserStatus?> build() async => null;
}

class _Config extends RelayConfigNotifier {
  @override
  RelayConfig build() => RelayConfig(
    baseUrl: 'https://relay.test',
    nsec: nostr.Keys(
      '1111111111111111111111111111111111111111111111111111111111111111',
    ).nsec,
  );
}

Future<void> _capture(WidgetTester tester, String name) async {
  final directory = Platform.environment['SETTINGS_SCREENSHOTS'];
  if (directory == null) return;
  final boundary = tester.renderObject<RenderRepaintBoundary>(
    find.byKey(const ValueKey('settings-test-capture')),
  );
  await tester.runAsync(() async {
    final image = await boundary.toImage(pixelRatio: 2);
    final bytes = await image.toByteData(format: ui.ImageByteFormat.png);
    await Directory(directory).create(recursive: true);
    await File(
      '$directory/$name.png',
    ).writeAsBytes(bytes!.buffer.asUint8List());
    image.dispose();
  });
}

class _Lifecycle extends AppLifecycleNotifier {
  @override
  AppLifecycleState build() => AppLifecycleState.resumed;
}
