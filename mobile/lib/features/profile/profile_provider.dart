import 'dart:async';
import 'dart:convert';

import 'package:flutter/widgets.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:http/http.dart' as http;
import 'package:nostr/nostr.dart' as nostr;

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

  Future<void> saveDisplayName(String displayName) async {
    final normalized = displayName.trim();
    if (normalized.isEmpty) {
      throw ArgumentError.value(
        displayName,
        'displayName',
        'must not be empty',
      );
    }

    final config = ref.read(relayConfigProvider);
    final nsec = config.nsec;
    if (nsec == null || nsec.isEmpty) {
      throw StateError('Cannot save profile without a signing key');
    }

    final previous = state.asData?.value;
    final client = http.Client();
    try {
      await publishProfileOverHttp(
        client: client,
        relayUrl: config.baseUrl,
        nsec: nsec,
        displayName: normalized,
        existing: previous,
      );
    } finally {
      client.close();
    }

    final pubkey = pubkeyFromNsec(nsec);
    if (pubkey == null) {
      throw StateError('Cannot derive profile public key');
    }
    state = AsyncData(
      UserProfile(
        pubkey: pubkey,
        displayName: normalized,
        avatarUrl: previous?.avatarUrl,
        about: previous?.about,
        nip05Handle: previous?.nip05Handle,
        ownerPubkey: previous?.ownerPubkey,
      ),
    );
  }
}

final profileProvider = AsyncNotifierProvider<ProfileNotifier, UserProfile?>(
  ProfileNotifier.new,
);

/// Publish a signed kind:0 profile through the relay's authenticated HTTP
/// bridge. Invite onboarding uses this before the WebSocket reconnect settles;
/// Settings uses the same path so an unnamed mobile identity can recover.
Future<void> publishProfileOverHttp({
  required http.Client client,
  required String relayUrl,
  required String nsec,
  required String displayName,
  UserProfile? existing,
}) async {
  final normalized = displayName.trim();
  if (normalized.isEmpty) {
    throw ArgumentError.value(displayName, 'displayName', 'must not be empty');
  }

  final privateKey = nostr.Nip19.decode(payload: nsec).data;
  if (privateKey.isEmpty) throw StateError('Invalid signing key');

  final content = <String, dynamic>{
    'name': normalized,
    'display_name': normalized,
    'picture': ?existing?.avatarUrl,
    'about': ?existing?.about,
    'nip05': ?existing?.nip05Handle,
  };
  final event = nostr.Event.from(
    kind: EventKind.metadata,
    content: jsonEncode(content),
    tags: const [],
    secretKey: privateKey,
    verify: false,
  );
  final bodyBytes = utf8.encode(jsonEncode(event.toMap()));
  final url = _eventsUrlFromRelay(relayUrl);
  final request = http.Request('POST', Uri.parse(url))
    ..followRedirects = false
    ..headers.addAll({
      'Authorization': buildNip98AuthHeader(
        method: 'POST',
        url: url,
        bodyBytes: bodyBytes,
        nsec: nsec,
      ),
      'Content-Type': 'application/json',
    })
    ..bodyBytes = bodyBytes;
  final response = await http.Response.fromStream(await client.send(request));
  final decoded = jsonDecode(response.body.isEmpty ? '{}' : response.body);
  final accepted =
      response.statusCode >= 200 &&
      response.statusCode < 300 &&
      decoded is Map &&
      decoded['accepted'] == true;
  if (!accepted) {
    throw StateError('Relay rejected profile update');
  }
}

String _eventsUrlFromRelay(String relayUrl) {
  final uri = Uri.parse(relayUrl);
  final scheme = switch (uri.scheme) {
    'wss' => 'https',
    'ws' => 'http',
    'https' => 'https',
    'http' => 'http',
    _ => throw FormatException('Invalid relay URL scheme: ${uri.scheme}'),
  };
  return Uri(
    scheme: scheme,
    host: uri.host,
    port: uri.hasPort ? uri.port : null,
    path: '/events',
  ).toString();
}

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
