import 'dart:async';
import 'dart:convert';
import 'dart:math';

import 'package:flutter/widgets.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/crypto/nip_oa.dart';
import '../../shared/relay/relay.dart';
import 'user_cache_provider.dart';
import 'user_profile.dart';

const maxProfileDisplayNameLength = 255;

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
      ownerPubkey: verifiedOaOwnerPubkey(events.first.tags, data.pubkey),
    );
  }

  Future<void> refresh() async {
    state = await AsyncValue.guard(_fetch);
  }

  /// Publish a replacement kind-0 event while preserving the current metadata.
  Future<void> updateDisplayName(String displayName) async {
    final trimmedName = displayName.trim();
    if (trimmedName.isEmpty) {
      throw ArgumentError.value(
        displayName,
        'displayName',
        'must not be blank',
      );
    }
    if (trimmedName.runes.length > maxProfileDisplayNameLength) {
      throw ArgumentError.value(
        displayName,
        'displayName',
        'must be $maxProfileDisplayNameLength characters or fewer',
      );
    }

    final operationConfig = ref.read(relayConfigProvider);
    final myPk = ref.read(myPubkeyProvider);
    if (myPk == null) {
      throw StateError('Cannot update profile without a signing key');
    }

    final session = ref.read(relaySessionProvider.notifier);
    final events = await session.fetchHistory(NostrFilters.profile(myPk));
    _ensureRelayScope(operationConfig, myPk);
    final currentEvent = events.firstOrNull;
    final metadata = _metadataFrom(currentEvent);
    metadata['display_name'] = trimmedName;
    final createdAt = max(
      DateTime.now().millisecondsSinceEpoch ~/ 1000,
      (currentEvent?.createdAt ?? -1) + 1,
    );

    NostrEvent? submittedEvent;
    await SignedEventRelay(session: session, nsec: operationConfig.nsec).submit(
      kind: EventKind.profileMetadata,
      content: jsonEncode(metadata),
      tags: currentEvent?.tags ?? const [],
      createdAt: createdAt,
      onSigned: (event) => submittedEvent = event,
    );
    _ensureRelayScope(operationConfig, myPk);
    final confirmedEvents = await session.fetchHistory(
      NostrFilters.profile(myPk),
    );
    _ensureRelayScope(operationConfig, myPk);
    if (confirmedEvents.firstOrNull?.id != submittedEvent?.id) {
      throw StateError('Profile update was superseded by another event');
    }

    final updated = UserProfile(
      pubkey: myPk.toLowerCase(),
      displayName: trimmedName,
      avatarUrl: _stringValue(metadata, 'picture'),
      about: _stringValue(metadata, 'about'),
      nip05Handle: _stringValue(metadata, 'nip05'),
      ownerPubkey: verifiedOaOwnerPubkey(currentEvent?.tags ?? const [], myPk),
    );
    state = AsyncData(updated);
    ref.read(userCacheProvider.notifier).updateProfile(updated);
  }

  void _ensureRelayScope(RelayConfig operationConfig, String pubkey) {
    final currentConfig = ref.read(relayConfigProvider);
    if (currentConfig.baseUrl != operationConfig.baseUrl ||
        currentConfig.nsec != operationConfig.nsec ||
        ref.read(myPubkeyProvider) != pubkey) {
      throw StateError('Active community changed during profile update');
    }
  }
}

Map<String, dynamic> _metadataFrom(NostrEvent? event) {
  if (event == null) return {};

  try {
    final decoded = jsonDecode(event.content);
    return decoded is Map<String, dynamic>
        ? Map<String, dynamic>.from(decoded)
        : {};
  } catch (_) {
    return {};
  }
}

String? _stringValue(Map<String, dynamic> metadata, String key) {
  final value = metadata[key];
  return value is String && value.isNotEmpty ? value : null;
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

  Timer? _heartbeatTimer;

  @override
  Future<String> build() {
    ref.watch(relaySessionProvider);
    ref.watch(profileProvider);

    final lifecycle = ref.watch(appLifecycleProvider);

    ref.onDispose(() {
      _heartbeatTimer?.cancel();
      _heartbeatTimer = null;
    });

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
