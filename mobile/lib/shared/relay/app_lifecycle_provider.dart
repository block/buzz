import 'dart:async';

import 'package:connectivity_plus/connectivity_plus.dart';
import 'package:flutter/widgets.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import 'relay_session.dart';

/// Tracks the app lifecycle state and drives websocket connect/disconnect
/// behavior for mobile battery efficiency.
class AppLifecycleNotifier extends Notifier<AppLifecycleState> {
  AppLifecycleListener? _listener;
  StreamSubscription<List<ConnectivityResult>>? _connectivitySub;

  @override
  AppLifecycleState build() {
    _listener = AppLifecycleListener(onStateChange: _onStateChange);

    // Watch connectivity changes to trigger reconnect on network restore.
    _connectivitySub = Connectivity().onConnectivityChanged.listen((results) {
      final hasNetwork = results.any((r) => r != ConnectivityResult.none);
      if (hasNetwork && state == AppLifecycleState.resumed) {
        ref.read(relaySessionProvider.notifier).onAppResumed();
      }
    });

    ref.onDispose(() {
      _listener?.dispose();
      _connectivitySub?.cancel();
    });

    return AppLifecycleState.resumed;
  }

  void _onStateChange(AppLifecycleState newState) {
    state = newState;

    final session = ref.read(relaySessionProvider.notifier);
    switch (newState) {
      case AppLifecycleState.resumed:
        session.onAppResumed();
      case AppLifecycleState.paused:
      case AppLifecycleState.detached:
        session.onAppPaused();
      case AppLifecycleState.inactive:
      case AppLifecycleState.hidden:
        break; // Brief transition states — no action.
    }
  }
}

final appLifecycleProvider =
    NotifierProvider<AppLifecycleNotifier, AppLifecycleState>(
      AppLifecycleNotifier.new,
    );

/// Whether the app is in the foreground. Unlike [appLifecycleProvider], it
/// ignores `inactive`: pulling down Control Center or Notification Center, a
/// system alert or a glance at the app switcher passes through `inactive` and
/// back to `resumed` without leaving the foreground, and is not a resume.
class AppForegroundNotifier extends Notifier<bool> {
  @override
  bool build() {
    ref.listen(appLifecycleProvider, (_, next) {
      if (next != AppLifecycleState.inactive) {
        state = next == AppLifecycleState.resumed;
      }
    });
    return switch (ref.read(appLifecycleProvider)) {
      AppLifecycleState.resumed || AppLifecycleState.inactive => true,
      _ => false,
    };
  }
}

final appForegroundProvider = NotifierProvider<AppForegroundNotifier, bool>(
  AppForegroundNotifier.new,
);
