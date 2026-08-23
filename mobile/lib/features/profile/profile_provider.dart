import 'dart:async';
import 'dart:convert';

import 'package:flutter/widgets.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/profile/user_cache_provider.dart';
import '../../shared/profile/user_profile.dart';
import '../../shared/relay/relay.dart';
import '../../shared/theme/theme.dart';

/// The current user's profile (kind:0 metadata) loaded over the relay
/// WebSocket. Returns null when no nsec is configured or when the user has
/// not yet published a profile.
class ProfileNotifier extends AsyncNotifier<UserProfile?> {
  Map<String, dynamic> _metadata = {};
  bool _hasHydrated = false;
  int _lastCreatedAt = 0;

  @override
  Future<UserProfile?> build() {
    ref.watch(relayConfigProvider);
    ref.watch(relaySessionProvider);
    _hasHydrated = false;
    return _fetch();
  }

  Future<UserProfile?> _fetch() async {
    final myPk = ref.read(myPubkeyProvider);
    if (myPk == null) {
      _metadata = {};
      _lastCreatedAt = 0;
      _hasHydrated = true;
      return null;
    }

    final session = ref.read(relaySessionProvider.notifier);
    final events = await session.fetchHistory(NostrFilters.profile(myPk));
    if (events.isEmpty) {
      _metadata = {};
      _lastCreatedAt = 0;
      _hasHydrated = true;
      return null;
    }
    final latest = events.reduce((current, event) {
      if (event.createdAt != current.createdAt) {
        return event.createdAt > current.createdAt ? event : current;
      }
      // Match the relay replacement tie-breaker: the lowest event id wins.
      return event.id.compareTo(current.id) < 0 ? event : current;
    });
    final decoded = jsonDecode(latest.content);
    if (decoded is! Map<String, dynamic>) {
      throw const FormatException('Profile metadata must be a JSON object.');
    }
    _metadata = Map<String, dynamic>.from(decoded);
    _lastCreatedAt = latest.createdAt;
    final data = ProfileData.fromEvent(latest);
    final profile = UserProfile(
      pubkey: data.pubkey,
      displayName: data.displayName,
      avatarUrl: data.avatarUrl,
      about: data.about,
      nip05Handle: data.nip05,
    );
    _hasHydrated = true;
    return profile;
  }

  Future<void> refresh() async {
    _hasHydrated = false;
    state = await AsyncValue.guard(_fetch);
  }

  /// Updates the current user's display name while preserving the other
  /// metadata fields in their latest kind:0 profile event.
  Future<void> updateDisplayName(String displayName) =>
      _publishProfilePatch({'display_name': displayName.trim()});

  /// Updates the current user's profile description.
  Future<void> updateAbout(String about) =>
      _publishProfilePatch({'about': about.trim()});

  /// Updates the current user's profile photo URL.
  Future<void> updateAvatarUrl(String avatarUrl) =>
      _publishProfilePatch({'picture': avatarUrl.trim()});

  Future<void> _publishProfilePatch(Map<String, dynamic> patch) async {
    if (!_hasHydrated || !state.hasValue) {
      throw StateError('Cannot update profile before metadata is loaded.');
    }
    final pubkey = ref.read(myPubkeyProvider);
    if (pubkey == null) {
      throw StateError('Cannot update profile without a signing identity.');
    }

    final nextMetadata = {..._metadata, ...patch};
    final config = ref.read(relayConfigProvider);
    final relay = SignedEventRelay(
      session: ref.read(relaySessionProvider.notifier),
      nsec: config.nsec,
    );
    final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
    final createdAt = now > _lastCreatedAt ? now : _lastCreatedAt + 1;
    await relay.submit(
      kind: EventKind.profile,
      content: jsonEncode(nextMetadata),
      tags: const [],
      createdAt: createdAt,
    );

    _metadata = nextMetadata;
    _lastCreatedAt = createdAt;
    final profile = UserProfile(
      pubkey: pubkey,
      displayName:
          _metadata['display_name'] as String? ?? _metadata['name'] as String?,
      avatarUrl: _metadata['picture'] as String?,
      about: _metadata['about'] as String?,
      nip05Handle: _metadata['nip05'] as String?,
    );
    state = AsyncData(profile);
    ref.read(userCacheProvider.notifier).put(profile);
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
