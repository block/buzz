import '../storage/serial_queue.dart';
import 'dart:convert';

import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:nostr/nostr.dart' as nostr;

import '../community/community.dart';
import '../relay/relay_socket.dart';
import '../storage/secure_storage.dart';
import 'account_identity_store.dart';
import 'community_entry_requirements.dart';
import 'device_identity_source.dart';

/// A self-hosted attempt keeps its key before any network operation. Retrying
/// after a refused connection or process death never rotates that identity.
class SelfHostedEntry {
  SelfHostedEntry({
    FlutterSecureStorage? storage,
    DeviceIdentitySource? identities,
    AccountIdentityStore? accountStore,
    Future<void> Function(Community)? probe,
  }) : _storage = storage ?? communitySecureStorage,
       identities = identities ?? DeviceIdentitySource(),
       _accountStore = accountStore ?? AccountIdentityStore(),
       _probe = probe ?? probeRelay;

  final FlutterSecureStorage _storage;
  final DeviceIdentitySource identities;
  final AccountIdentityStore _accountStore;
  final Future<void> Function(Community) _probe;
  static final _queue = SerialQueue();

  /// Saved keys available for explicit reuse, including a hosted account that
  /// has not joined any community yet. This performs no account network calls.
  Future<({List<DeviceIdentityChoice> choices, bool complete})>
  choices() async {
    var complete = true;
    var device = <DeviceIdentityChoice>[];
    var accounts = <AccountIdentitySnapshot>[];
    try {
      device = await identities.choices();
    } on AccountIdentityFailure {
      complete = false;
    }
    try {
      accounts = await _accountStore.identities();
    } on AccountIdentityFailure {
      complete = false;
    }
    return (
      choices: [
        ...device,
        for (final account in accounts)
          if (!device.any((item) => item.pubkey == account.pubkey))
            DeviceIdentityChoice(account.pubkey, 'Saved identity'),
      ],
      complete: complete,
    );
  }

  Future<Community> prepare({
    required String relay,
    required String name,
    String? nsec,
    String? savedPubkey,
    bool create = false,
    required bool Function() isCurrent,
  }) {
    final result = _queue.run(() async {
      CommunityEntryRequirements.requireSelfHosted(relay);
      final uri = Uri.parse(relay);
      if (uri.scheme != 'wss' ||
          uri.host.isEmpty ||
          uri.userInfo.isNotEmpty ||
          uri.hasQuery ||
          uri.hasFragment ||
          (uri.path.isNotEmpty && uri.path != '/')) {
        throw const FormatException('Invalid community address');
      }
      void check() {
        if (!isCurrent()) throw StateError('Setup cancelled');
      }

      check();
      final key = _pendingKey(relay);
      final retained = await _storage.read(key: key);
      String? candidate = nsec;
      if (savedPubkey != null) {
        // Selection is explicit: resolve the exact key without requiring a
        // complete inventory of unrelated account slots.
        try {
          candidate = await identities.withIdentity(
            savedPubkey,
            (nsec) async => nsec,
          );
        } on AccountIdentityFailure {
          candidate = await _accountStore.withSavedIdentity(
            savedPubkey,
            (nsec) async => nsec,
          );
        }
      }
      if (retained != null) {
        final retainedPubkey = AccountIdentityStore.publicKeyForNsec(retained);
        if (candidate != null &&
            AccountIdentityStore.publicKeyForNsec(candidate) !=
                retainedPubkey) {
          // An explicit replacement may follow a refused or uncertain probe.
          // Preserve the previous key before changing the relay's retry slot.
          await _accountStore.forPairing(retainedPubkey).create(retained);
          check();
        }
        candidate ??= retained;
      }
      if (candidate == null) {
        if (!create) throw AccountIdentityFailure.absent;
        final inventory = await choices();
        if (!inventory.complete) throw AccountIdentityFailure.unreadable;
        if (inventory.choices.isNotEmpty) {
          throw AccountIdentityFailure.absent;
        }
        candidate = nostr.Keys.generate().nsec;
      }
      final pubkey = AccountIdentityStore.publicKeyForNsec(candidate);
      check();
      if (retained != candidate) {
        await _storage.write(key: key, value: candidate);
        if (await _storage.read(key: key) != candidate) {
          throw AccountIdentityFailure.unreadable;
        }
      }
      check();
      final community = Community.create(
        name: name,
        relayUrl: relay,
        pubkey: pubkey,
        nsec: candidate,
      );
      await _probe(community);
      check();
      return community;
    });
    return result;
  }

  static String _pendingKey(String relay) =>
      'buzz_pending_self_hosted_${base64Url.encode(utf8.encode(relay))}';

  /// Retire this exact pending key only after its community is durably saved.
  Future<void> complete(Community community) => _queue.run(() async {
    final key = _pendingKey(community.relayUrl);
    if (await _storage.read(key: key) != community.nsec) return;
    await _storage.delete(key: key);
    if (await _storage.read(key: key) != null) {
      throw AccountIdentityFailure.unreadable;
    }
  });

  /// Authentication must succeed before a new community becomes active.
  static Future<void> probeRelay(Community community) async {
    final socket = RelaySocket(
      wsUrl: community.relayUrl,
      nsec: community.nsec,
      onMessage: (_) {},
      onConnected: () {},
      onDisconnected: (_) {},
    );
    try {
      await socket.connect().timeout(const Duration(seconds: 20));
      if (socket.state != SocketState.connected) {
        throw StateError('Community refused entry');
      }
    } finally {
      socket.dispose();
    }
  }
}
