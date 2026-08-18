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
