import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';

import '../relay/relay.dart';
import '../theme/theme_provider.dart';
import 'android_notification_bridge.dart';

const _prefsPrefix = 'android_notification_settings_v1';

@immutable
class NotificationSettingsState {
  const NotificationSettingsState({
    this.alertsEnabled = false,
    this.priorityEnabled = true,
    this.activityEnabled = false,
    this.previewsEnabled = false,
    this.permission = AndroidNotificationPermission.notDetermined,
    this.priorityChannelEnabled = false,
    this.activityChannelEnabled = false,
    this.isRequesting = false,
  });

  final bool alertsEnabled;
  final bool priorityEnabled;
  final bool activityEnabled;
  final bool previewsEnabled;
  final AndroidNotificationPermission permission;
  final bool priorityChannelEnabled;
  final bool activityChannelEnabled;
  final bool isRequesting;

  NotificationSettingsState copyWith({
    bool? alertsEnabled,
    bool? priorityEnabled,
    bool? activityEnabled,
    bool? previewsEnabled,
    AndroidNotificationPermission? permission,
    bool? priorityChannelEnabled,
    bool? activityChannelEnabled,
    bool? isRequesting,
  }) {
    return NotificationSettingsState(
      alertsEnabled: alertsEnabled ?? this.alertsEnabled,
      priorityEnabled: priorityEnabled ?? this.priorityEnabled,
      activityEnabled: activityEnabled ?? this.activityEnabled,
      previewsEnabled: previewsEnabled ?? this.previewsEnabled,
      permission: permission ?? this.permission,
      priorityChannelEnabled:
          priorityChannelEnabled ?? this.priorityChannelEnabled,
      activityChannelEnabled:
          activityChannelEnabled ?? this.activityChannelEnabled,
      isRequesting: isRequesting ?? this.isRequesting,
    );
  }
}

class NotificationSettingsNotifier extends Notifier<NotificationSettingsState> {
  late String _prefsKey;
  int _refreshGeneration = 0;
  int _scopeGeneration = 0;
  int _alertsMutationGeneration = 0;
  bool _permissionRequestInFlight = false;

  @override
  NotificationSettingsState build() {
    final scopeGeneration = ++_scopeGeneration;
    final config = ref.watch(relayConfigProvider);
    final pubkey = ref.watch(myPubkeyProvider) ?? 'anon';
    ref.watch(relaySessionProvider.select((state) => state.status));
    if (defaultTargetPlatform == TargetPlatform.android) {
      ref.listen(appLifecycleProvider, (previous, next) {
        if (previous != AppLifecycleState.resumed &&
            next == AppLifecycleState.resumed) {
          final generation = ++_refreshGeneration;
          Future.microtask(() => _refreshStatus(generation, scopeGeneration));
        }
      });
    }
    _prefsKey = '$_prefsPrefix:${config.baseUrl}:$pubkey';
    final legacyPrefsKey = '$_prefsPrefix:${config.storedOrigin}:$pubkey';

    final prefs = ref.read(savedPrefsProvider);
    final state = NotificationSettingsState(
      alertsEnabled:
          _readPreference(prefs, 'alerts', legacyPrefsKey: legacyPrefsKey) ??
          false,
      priorityEnabled:
          _readPreference(prefs, 'priority', legacyPrefsKey: legacyPrefsKey) ??
          true,
      activityEnabled:
          _readPreference(prefs, 'activity', legacyPrefsKey: legacyPrefsKey) ??
          false,
      previewsEnabled:
          _readPreference(prefs, 'previews', legacyPrefsKey: legacyPrefsKey) ??
          false,
    );

    if (defaultTargetPlatform == TargetPlatform.android) {
      final statusSubscription = ref
          .read(androidNotificationBridgeProvider)
          .statusChanges
          .listen((status) {
            if (!ref.mounted) return;
            this.state = this.state.copyWith(
              permission: status.permission,
              priorityChannelEnabled: status.priorityChannelEnabled,
              activityChannelEnabled: status.activityChannelEnabled,
            );
          });
      ref.onDispose(statusSubscription.cancel);
      final generation = ++_refreshGeneration;
      Future.microtask(() => _refreshStatus(generation, scopeGeneration));
    }
    return state;
  }

  Future<void> setAlertsEnabled(bool enabled) async {
    if (!enabled) {
      _alertsMutationGeneration++;
      _persist(alerts: false);
      state = state.copyWith(alertsEnabled: false, isRequesting: false);
      return;
    }
    if (defaultTargetPlatform != TargetPlatform.android) return;
    if (_permissionRequestInFlight) return;

    // An initial/resume refresh may still be reading the pre-prompt status.
    // Once the explicit permission flow starts, that older result is stale.
    _refreshGeneration++;
    final scopeGeneration = _scopeGeneration;
    final mutationGeneration = ++_alertsMutationGeneration;
    _permissionRequestInFlight = true;
    state = state.copyWith(isRequesting: true);
    try {
      final bridge = ref.read(androidNotificationBridgeProvider);
      var status = await bridge.requestPermission();
      if (!_canApplyPermissionResult(scopeGeneration, mutationGeneration)) {
        return;
      }
      if (status.permission == AndroidNotificationPermission.granted) {
        status = await bridge.ensureChannels();
        if (!_canApplyPermissionResult(scopeGeneration, mutationGeneration)) {
          return;
        }
        _persist(alerts: true);
      }
      state = state.copyWith(
        alertsEnabled:
            status.permission == AndroidNotificationPermission.granted,
        permission: status.permission,
        priorityChannelEnabled: status.priorityChannelEnabled,
        activityChannelEnabled: status.activityChannelEnabled,
      );
    } on MissingPluginException {
      // The native half may not be present in test/development builds.
    } on PlatformException {
      // A native failure must not turn the persisted master preference on.
    } finally {
      _permissionRequestInFlight = false;
      if (_canApplyPermissionResult(scopeGeneration, mutationGeneration)) {
        state = state.copyWith(isRequesting: false);
      }
    }
  }

  void setPriorityEnabled(bool enabled) {
    _persist(priority: enabled);
    state = state.copyWith(priorityEnabled: enabled);
  }

  void setActivityEnabled(bool enabled) {
    _persist(activity: enabled);
    state = state.copyWith(activityEnabled: enabled);
  }

  void setPreviewsEnabled(bool enabled) {
    _persist(previews: enabled);
    state = state.copyWith(previewsEnabled: enabled);
  }

  Future<void> refreshStatus() =>
      _refreshStatus(++_refreshGeneration, _scopeGeneration);

  Future<void> openSettings() async {
    if (defaultTargetPlatform != TargetPlatform.android) return;
    await ref.read(androidNotificationBridgeProvider).openSettings();
  }

  Future<void> _refreshStatus(int generation, int scopeGeneration) async {
    try {
      final status = await ref
          .read(androidNotificationBridgeProvider)
          .getStatus();
      if (!ref.mounted ||
          generation != _refreshGeneration ||
          scopeGeneration != _scopeGeneration) {
        return;
      }
      state = state.copyWith(
        permission: status.permission,
        priorityChannelEnabled: status.priorityChannelEnabled,
        activityChannelEnabled: status.activityChannelEnabled,
      );
    } on MissingPluginException {
      // Non-native test and development surfaces have no Android implementation.
    } on PlatformException {
      // The Dart slice can land before its native peer. Keep the conservative
      // initial status instead of presenting alerts as available.
    }
  }

  bool _canApplyPermissionResult(int scopeGeneration, int mutationGeneration) =>
      ref.mounted &&
      scopeGeneration == _scopeGeneration &&
      mutationGeneration == _alertsMutationGeneration;

  void _persist({
    bool? alerts,
    bool? priority,
    bool? activity,
    bool? previews,
  }) {
    final prefs = ref.read(savedPrefsProvider);
    if (alerts != null) prefs.setBool('$_prefsKey:alerts', alerts);
    if (priority != null) prefs.setBool('$_prefsKey:priority', priority);
    if (activity != null) prefs.setBool('$_prefsKey:activity', activity);
    if (previews != null) prefs.setBool('$_prefsKey:previews', previews);
  }

  bool? _readPreference(
    SharedPreferences prefs,
    String suffix, {
    required String legacyPrefsKey,
  }) {
    return readMigratedPref<bool>(
      prefs,
      canonicalKey: '$_prefsKey:$suffix',
      legacyKey: '$legacyPrefsKey:$suffix',
      read: prefs.getBool,
      write: prefs.setBool,
    );
  }
}

final notificationSettingsProvider =
    NotifierProvider<NotificationSettingsNotifier, NotificationSettingsState>(
      NotificationSettingsNotifier.new,
    );
