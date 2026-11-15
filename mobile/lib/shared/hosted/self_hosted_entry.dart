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
    bool alongsideSaved = false,
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
      final generatedKey = _generatedKey(relay);
      final retained = await _storage.read(key: key);
      String? candidate = nsec;
      // A retained key for this exact identity is the one an interrupted
      // attempt chose; it may exist nowhere else yet.
      if (savedPubkey != null &&
          (retained == null ||
              AccountIdentityStore.publicKeyForNsec(retained) != savedPubkey)) {
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
        } else if (candidate == null && create && alongsideSaved) {
          // The member asked for a new identity, so reuse only a key an
          // interrupted Create generated here. Any other retained key (a saved
          // or imported one) stays saved, and a fresh key is generated.
          if (await _storage.read(key: generatedKey) == retainedPubkey) {
            candidate = retained;
          } else {
            await _accountStore.forPairing(retainedPubkey).create(retained);
          }
          check();
        } else {
          candidate ??= retained;
        }
      }
      var generated = false;
      if (candidate == null) {
        if (!create) throw AccountIdentityFailure.absent;
        if (!alongsideSaved) {
          // Unprompted creation: saved identities need the member's choice.
          final inventory = await choices();
          if (!inventory.complete) throw AccountIdentityFailure.unreadable;
          if (inventory.choices.isNotEmpty) {
            throw AccountIdentityFailure.absent;
          }
        }
        candidate = nostr.Keys.generate().nsec;
        generated = true;
      }
      final pubkey = AccountIdentityStore.publicKeyForNsec(candidate);
      check();
      if (retained != candidate) {
        if (!generated && await _storage.read(key: generatedKey) != null) {
          await _storage.delete(key: generatedKey);
        }
        await _storage.write(key: key, value: candidate);
        if (await _storage.read(key: key) != candidate) {
          throw AccountIdentityFailure.unreadable;
        }
        // Written after the key: a missing marker only costs a fresh key.
        if (generated) await _storage.write(key: generatedKey, value: pubkey);
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

  /// The public key of a pending key that Create generated for this relay.
  static String _generatedKey(String relay) =>
      '${_pendingKey(relay)}_generated';

  /// Retire this exact pending key only after its community is durably saved.
  Future<void> complete(Community community) => _queue.run(() async {
    final key = _pendingKey(community.relayUrl);
    if (await _storage.read(key: key) != community.nsec) return;
    await _storage.delete(key: _generatedKey(community.relayUrl));
    await _storage.delete(key: key);
    if (await _storage.read(key: key) != null) {
      throw AccountIdentityFailure.unreadable;
    }
  });

  /// Authentication must succeed before a new community becomes active.
  /// [headers] go on the upgrade, for a relay that requires an assertion.
  static Future<void> probeRelay(
    Community community, [
    Map<String, String> headers = const {},
  ]) async {
    final socket = RelaySocket(
      wsUrl: community.relayUrl,
      nsec: community.nsec,
      onMessage: (_) {},
      onConnected: () {},
      onDisconnected: (_) {},
    );
    try {
      socket.headers = headers;
      await socket.connect().timeout(const Duration(seconds: 20));
      if (socket.state != SocketState.connected) {
        throw StateError('Community refused entry');
      }
    } finally {
      socket.dispose();
    }
  }
}
