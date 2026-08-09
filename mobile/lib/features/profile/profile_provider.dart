import 'dart:async';

import 'package:flutter/widgets.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/relay/relay.dart';
import '../../shared/theme/theme.dart';
import 'user_profile.dart';

/// The current user's profile (kind:0 metadata) loaded over the relay
/// WebSocket. Returns null when no nsec is configured or when the user has
/// not yet published a profile.
class ProfileNotifier extends AsyncNotifier<UserProfile?> {
  @override
  Future<UserProfile?> build() {
    ref.watch(relayConfigProvider);
    ref.watch(relaySessionProvider);
    return _fetch();
  }

  Future<UserProfile?> _fetch() async {
    final myPk = ref.read(myPubkeyProvider);
    if (myPk == null) return null;

    final session = ref.read(relaySessionProvider.notifier);
    final events = await session.fetchHistory(NostrFilters.profile(myPk));
    if (events.isEmpty) return null;
    final data = ProfileData.fromEvent(events.first);
    return UserProfile(
      pubkey: data.pubkey,
      displayName: data.displayName,
      avatarUrl: data.avatarUrl,
      about: data.about,
      nip05Handle: data.nip05,
    );
  }

  Future<void> refresh() async {
    state = await AsyncValue.guard(_fetch);
  }
}

final profileProvider = AsyncNotifierProvider<ProfileNotifier, UserProfile?>(
  ProfileNotifier.new,
);

/// Presence status for the current user.
///
/// Sends a heartbeat every 60s while the app is active by publishing a
/// kind:20001 presence event over the relay WebSocket. Watches
/// [appLifecycleProvider] to send "away" when backgrounded.
class PresenceNotifier extends AsyncNotifier<String> {
  static const _heartbeatInterval = Duration(seconds: 60);
  static const _preferenceKeyPrefix = 'buzz_presence_preference_';

  Timer? _heartbeatTimer;
  String? _preferencePubkey;
  String? _manualPresence;

  @override
  Future<String> build() {
    ref.watch(relaySessionProvider);
    final pubkey = ref.watch(myPubkeyProvider)?.toLowerCase();

    if (_preferencePubkey != pubkey) {
      _preferencePubkey = pubkey;
      final stored = pubkey == null
          ? null
          : ref
                .read(savedPrefsProvider)
                .getString('$_preferenceKeyPrefix$pubkey');
      _manualPresence = stored == 'away' || stored == 'offline' ? stored : null;
    }

    final lifecycle = ref.watch(appLifecycleProvider);

    ref.onDispose(() {
      _heartbeatTimer?.cancel();
      _heartbeatTimer = null;
    });

    final manualPresence = _manualPresence;
    if (manualPresence != null) {
      _heartbeatTimer?.cancel();
      _heartbeatTimer = null;
      return _setPresence(manualPresence);
    }

    if (lifecycle == AppLifecycleState.resumed) {
      _startHeartbeat();
      return _setPresence('online');
    } else if (lifecycle == AppLifecycleState.paused ||
        lifecycle == AppLifecycleState.detached) {
      _heartbeatTimer?.cancel();
      _heartbeatTimer = null;
      return _setPresence('away');
    }

    // Default: we don't know. Reflect the most recent state we set, or
    // 'offline' if never set.
    return Future.value('offline');
  }

  void _startHeartbeat() {
    _heartbeatTimer?.cancel();
    _heartbeatTimer = Timer.periodic(_heartbeatInterval, (_) {
      _setPresence('online');
    });
  }

  /// Updates the current user's presence preference and publishes it.
  ///
  /// Online restores automatic lifecycle-driven presence. Away and Offline
  /// remain selected until the user chooses another value.
  Future<void> setPresence(String status) async {
    if (status != 'online' && status != 'away' && status != 'offline') return;

    _manualPresence = status == 'online' ? null : status;
    final pubkey = ref.read(myPubkeyProvider)?.toLowerCase();
    if (pubkey != null) {
      await ref
          .read(savedPrefsProvider)
          .setString('$_preferenceKeyPrefix$pubkey', _manualPresence ?? 'auto');
    }

    if (_manualPresence == null &&
        ref.read(appLifecycleProvider) == AppLifecycleState.resumed) {
      _startHeartbeat();
    } else {
      _heartbeatTimer?.cancel();
      _heartbeatTimer = null;
    }

    state = AsyncData(status);
    await _setPresence(status);
  }

  /// Publish a kind:20001 presence event. Returns the requested status
  /// optimistically — failures are silently absorbed and the next heartbeat
  /// will retry.
  Future<String> _setPresence(String status) async {
    final sessionState = ref.read(relaySessionProvider);
    if (sessionState.status != SessionStatus.connected) return status;
    final config = ref.read(relayConfigProvider);
    final relay = SignedEventRelay(
      session: ref.read(relaySessionProvider.notifier),
      nsec: config.nsec,
    );
    try {
      await relay.submit(
        kind: EventKind.presenceUpdate,
        content: status,
        tags: const [],
      );
    } catch (_) {
      // Heartbeat will retry.
    }
    return status;
  }

  Future<void> refresh() async {
    // No-op: presence is driven by heartbeats and lifecycle, not pulled.
  }
}

final presenceProvider = AsyncNotifierProvider<PresenceNotifier, String>(
  PresenceNotifier.new,
);
