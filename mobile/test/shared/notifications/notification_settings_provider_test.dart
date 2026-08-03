import 'dart:async';

import 'package:buzz/shared/notifications/notifications.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  late TargetPlatform? previousPlatform;

  setUp(() {
    previousPlatform = debugDefaultTargetPlatformOverride;
    debugDefaultTargetPlatformOverride = TargetPlatform.android;
  });

  tearDown(() {
    debugDefaultTargetPlatformOverride = previousPlatform;
  });

  Future<ProviderContainer> container({
    Map<String, Object> preferences = const {},
    AndroidNotificationStatus status = const AndroidNotificationStatus(
      permission: AndroidNotificationPermission.notDetermined,
      priorityChannelEnabled: true,
      activityChannelEnabled: true,
    ),
    _FakeBridge? fakeBridge,
  }) async {
    SharedPreferences.setMockInitialValues(preferences);
    final prefs = await SharedPreferences.getInstance();
    final bridge = fakeBridge ?? _FakeBridge(status);
    addTearDown(bridge.dispose);
    final result = ProviderContainer(
      overrides: [
        savedPrefsProvider.overrideWithValue(prefs),
        relayConfigProvider.overrideWith(
          () => _RelayConfigNotifier('https://relay.example'),
        ),
        relaySessionProvider.overrideWith(_DisconnectedSession.new),
        appLifecycleProvider.overrideWith(_FakeLifecycle.new),
        androidNotificationBridgeProvider.overrideWithValue(bridge),
      ],
    );
    addTearDown(result.dispose);
    return result;
  }

  test('defaults master off, priority on, and activity off', () async {
    final scope = await container();
    final bridge = scope.read(androidNotificationBridgeProvider) as _FakeBridge;

    final state = scope.read(notificationSettingsProvider);
    await Future<void>.delayed(Duration.zero);

    expect(state.alertsEnabled, isFalse);
    expect(state.priorityEnabled, isTrue);
    expect(state.activityEnabled, isFalse);
    expect(state.previewsEnabled, isFalse);
    expect(bridge.permissionRequests, 0);
  });

  test('requests permission only on explicit master enable', () async {
    final scope = await container(
      status: const AndroidNotificationStatus(
        permission: AndroidNotificationPermission.granted,
        priorityChannelEnabled: true,
        activityChannelEnabled: true,
      ),
    );
    final bridge = scope.read(androidNotificationBridgeProvider) as _FakeBridge;
    scope.read(notificationSettingsProvider);
    await Future<void>.delayed(Duration.zero);

    await scope
        .read(notificationSettingsProvider.notifier)
        .setAlertsEnabled(true);

    expect(bridge.permissionRequests, 1);
    expect(bridge.channelEnsures, 1);
    expect(scope.read(notificationSettingsProvider).alertsEnabled, isTrue);
  });

  test('ignores a duplicate enable while Android is prompting', () async {
    final permissionCompleter = Completer<AndroidNotificationStatus>();
    final bridge = _FakeBridge(
      const AndroidNotificationStatus(
        permission: AndroidNotificationPermission.granted,
        priorityChannelEnabled: true,
        activityChannelEnabled: true,
      ),
      permissionCompleter: permissionCompleter,
    );
    final scope = await container(fakeBridge: bridge);
    final notifier = scope.read(notificationSettingsProvider.notifier);

    final first = notifier.setAlertsEnabled(true);
    await Future<void>.delayed(Duration.zero);
    expect(scope.read(notificationSettingsProvider).isRequesting, isTrue);

    await notifier.setAlertsEnabled(true);
    expect(bridge.permissionRequests, 1);

    permissionCompleter.complete(bridge.status);
    await first;
    expect(scope.read(notificationSettingsProvider).isRequesting, isFalse);
  });

  test('denied permission leaves the master disabled', () async {
    final scope = await container(
      status: const AndroidNotificationStatus(
        permission: AndroidNotificationPermission.denied,
        priorityChannelEnabled: false,
        activityChannelEnabled: false,
      ),
    );
    final bridge = scope.read(androidNotificationBridgeProvider) as _FakeBridge;

    await scope
        .read(notificationSettingsProvider.notifier)
        .setAlertsEnabled(true);

    expect(bridge.permissionRequests, 1);
    expect(bridge.channelEnsures, 0);
    expect(scope.read(notificationSettingsProvider).alertsEnabled, isFalse);
    expect(
      scope.read(notificationSettingsProvider).permission,
      AndroidNotificationPermission.denied,
    );
  });

  test(
    'disabling while permission is pending cannot re-enable alerts',
    () async {
      final permissionCompleter = Completer<AndroidNotificationStatus>();
      final bridge = _FakeBridge(
        const AndroidNotificationStatus(
          permission: AndroidNotificationPermission.granted,
          priorityChannelEnabled: true,
          activityChannelEnabled: true,
        ),
        permissionCompleter: permissionCompleter,
      );
      final scope = await container(fakeBridge: bridge);
      final notifier = scope.read(notificationSettingsProvider.notifier);

      final enabling = notifier.setAlertsEnabled(true);
      await Future<void>.delayed(Duration.zero);
      await notifier.setAlertsEnabled(false);
      permissionCompleter.complete(bridge.status);
      await enabling;

      expect(scope.read(notificationSettingsProvider).alertsEnabled, isFalse);
      expect(scope.read(notificationSettingsProvider).isRequesting, isFalse);
      expect(bridge.channelEnsures, 0);
    },
  );

  test('refreshes Android status when the app resumes', () async {
    final bridge = _FakeBridge(
      const AndroidNotificationStatus(
        permission: AndroidNotificationPermission.granted,
        priorityChannelEnabled: true,
        activityChannelEnabled: true,
      ),
    );
    final scope = await container(fakeBridge: bridge);
    scope.read(notificationSettingsProvider);
    await Future<void>.delayed(Duration.zero);
    bridge.status = const AndroidNotificationStatus(
      permission: AndroidNotificationPermission.denied,
      priorityChannelEnabled: false,
      activityChannelEnabled: false,
    );

    final lifecycle =
        scope.read(appLifecycleProvider.notifier) as _FakeLifecycle;
    lifecycle.setLifecycle(AppLifecycleState.paused);
    lifecycle.setLifecycle(AppLifecycleState.resumed);
    await Future<void>.delayed(Duration.zero);
    await Future<void>.delayed(Duration.zero);

    final state = scope.read(notificationSettingsProvider);
    expect(state.permission, AndroidNotificationPermission.denied);
    expect(state.priorityChannelEnabled, isFalse);
    expect(state.activityChannelEnabled, isFalse);
  });

  test('persists preferences under relay and identity scope', () async {
    final scope = await container();
    final notifier = scope.read(notificationSettingsProvider.notifier);
    notifier.setPriorityEnabled(false);
    notifier.setActivityEnabled(true);
    notifier.setPreviewsEnabled(true);

    final prefs = scope.read(savedPrefsProvider);
    expect(
      prefs.getBool(
        'android_notification_settings_v1:https://relay.example:anon:priority',
      ),
      isFalse,
    );
    expect(
      prefs.getBool(
        'android_notification_settings_v1:https://relay.example:anon:activity',
      ),
      isTrue,
    );
    expect(
      prefs.getBool(
        'android_notification_settings_v1:https://relay.example:anon:previews',
      ),
      isTrue,
    );
  });

  test('refreshes channel state from the native resume callback', () async {
    final bridge = _FakeBridge(
      const AndroidNotificationStatus(
        permission: AndroidNotificationPermission.granted,
        priorityChannelEnabled: true,
        activityChannelEnabled: true,
      ),
    );
    final scope = await container(fakeBridge: bridge);
    scope.read(notificationSettingsProvider);
    await Future<void>.delayed(Duration.zero);

    bridge.emitStatus(
      const AndroidNotificationStatus(
        permission: AndroidNotificationPermission.denied,
        priorityChannelEnabled: false,
        activityChannelEnabled: false,
      ),
    );
    await Future<void>.delayed(Duration.zero);

    final state = scope.read(notificationSettingsProvider);
    expect(state.permission, AndroidNotificationPermission.denied);
    expect(state.priorityChannelEnabled, isFalse);
    expect(state.activityChannelEnabled, isFalse);
  });
}

class _FakeBridge extends AndroidNotificationBridge {
  _FakeBridge(this.status, {this.permissionCompleter});

  AndroidNotificationStatus status;
  final Completer<AndroidNotificationStatus>? permissionCompleter;
  int permissionRequests = 0;
  int channelEnsures = 0;
  final StreamController<AndroidNotificationStatus> _statusController =
      StreamController<AndroidNotificationStatus>.broadcast();

  @override
  Stream<AndroidNotificationStatus> get statusChanges =>
      _statusController.stream;

  void emitStatus(AndroidNotificationStatus value) =>
      _statusController.add(value);

  @override
  Future<AndroidNotificationStatus> getStatus() async => status;

  @override
  Future<AndroidNotificationStatus> requestPermission() async {
    permissionRequests++;
    return permissionCompleter?.future ?? status;
  }

  @override
  Future<AndroidNotificationStatus> ensureChannels() async {
    channelEnsures++;
    return status;
  }

  @override
  void dispose() {
    _statusController.close();
    super.dispose();
  }
}

class _RelayConfigNotifier extends RelayConfigNotifier {
  _RelayConfigNotifier(this.url);

  final String url;

  @override
  RelayConfig build() => RelayConfig(baseUrl: url);
}

class _DisconnectedSession extends RelaySessionNotifier {
  @override
  SessionState build() =>
      const SessionState(status: SessionStatus.disconnected);
}

class _FakeLifecycle extends AppLifecycleNotifier {
  @override
  AppLifecycleState build() => AppLifecycleState.resumed;

  void setLifecycle(AppLifecycleState value) => state = value;
}
