import 'dart:convert';

import 'package:flutter/foundation.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';

import '../storage/serial_queue.dart';
import 'hosted_environment.dart';

/// One queued upload to the account's known-communities list, by the key the
/// membership was joined or left with: only that key's account sends it.
typedef KnownCommunityIntent = ({String url, bool removed, String pubkey});

/// A saved community: its relay URL and the public key it was joined with.
typedef KnownCommunityMembership = ({String relayUrl, String? pubkey});

/// The account service's spelling of a saved destination this build's account
/// session can enter, `wss://host`; null for any other relay.
String? knownCommunityAddress(String relayUrl) {
  final relay = Uri.tryParse(relayUrl);
  if (relay == null || !HostedEnvironment.supportsRelay(relay)) return null;
  return Uri(scheme: 'wss', host: relay.host.toLowerCase()).toString();
}

/// This device's record of the account's known-communities list, shared with
/// desktop's rule: [known] is everything the service has listed or accepted an
/// add for, less the removals it accepted from here, for the account named by
/// subject. So a membership another device removed is not uploaded again, and
/// removals made elsewhere are not followed. The outbox holds the latest
/// intent per destination and survives sign-out and account changes. Not
/// secret: destinations and public keys only, never private keys or
/// credentials.
class KnownCommunitiesStore extends ChangeNotifier {
  KnownCommunitiesStore({Future<SharedPreferences> Function()? preferences})
    : _preferences = preferences ?? SharedPreferences.getInstance;

  static const _key = 'buzz_known_communities_v1';
  final Future<SharedPreferences> Function() _preferences;
  final _queue = SerialQueue();
  String? _subject;
  Set<String> _known = {};
  // Insertion-ordered, oldest first.
  Map<String, ({bool removed, String pubkey})> _outbox = {};
  bool _loaded = false;

  /// Pending intents, oldest first.
  Future<List<KnownCommunityIntent>> pending() => _serial(
    () async => [
      for (final MapEntry(:key, :value) in _outbox.entries)
        (url: key, removed: value.removed, pubkey: value.pubkey),
    ],
  );

  /// Leaving always queues a removal, in case another device saved it.
  Future<void> left(String relayUrl, String? pubkey) async {
    final url = knownCommunityAddress(relayUrl);
    if (url == null || pubkey == null) return;
    await _serial(() async {
      _outbox.remove(url);
      _outbox[url] = (removed: true, pubkey: pubkey.toLowerCase());
      await _save();
    });
    notifyListeners();
  }

  /// Merges a complete list for [subject]. A different account's record starts
  /// from nothing known; its queued intents are kept.
  Future<void> listed(String subject, Iterable<String> urls) => _serial(() {
    if (_subject != subject) {
      _subject = subject;
      _known = {};
    }
    _known.addAll(urls);
    return _save();
  });

  /// Queues an add for each membership joined with [pubkey], the signed-in
  /// account's key, that the account is not known to hold, and replaces a
  /// pending removal of one joined again. [saved] is read inside the queue,
  /// so a removal queued after its leave is never undone by an older read of
  /// the saved list.
  Future<void> joined(
    String subject,
    String pubkey,
    Future<Iterable<KnownCommunityMembership>> Function() saved,
  ) => _serial(() async {
    if (_subject != subject) return;
    final key = pubkey.toLowerCase();
    var changed = false;
    for (final membership in await saved()) {
      final url = knownCommunityAddress(membership.relayUrl);
      if (url == null || membership.pubkey?.toLowerCase() != key) continue;
      final pending = _outbox[url];
      if ((pending != null && !pending.removed) ||
          (pending == null && _known.contains(url))) {
        continue;
      }
      _outbox.remove(url);
      _outbox[url] = (removed: false, pubkey: key);
      changed = true;
    }
    if (changed) await _save();
  });

  /// Records an accepted upload; the intent stays if a newer one replaced it.
  Future<void> accepted(String subject, KnownCommunityIntent intent) =>
      _serial(() {
        if (_subject == subject) {
          if (intent.removed) {
            _known.remove(intent.url);
          } else {
            _known.add(intent.url);
          }
        }
        if (_outbox[intent.url] ==
            (removed: intent.removed, pubkey: intent.pubkey)) {
          _outbox.remove(intent.url);
        }
        return _save();
      });

  Future<T> _serial<T>(Future<T> Function() operation) => _queue.run(() async {
    if (!_loaded) await _load();
    return operation();
  });

  Future<void> _load() async {
    final raw = (await _preferences()).getString(_key);
    _loaded = true;
    if (raw == null) return;
    try {
      final value = jsonDecode(raw) as Map<String, dynamic>;
      _subject = value['subject'] as String?;
      _known = {
        for (final url in value['known'] as List)
          if (url is String && knownCommunityAddress(url) == url) url,
      };
      _outbox = {
        for (final MapEntry(:key, :value)
            in (value['outbox'] as Map<String, dynamic>).entries)
          // A malformed entry is dropped on its own.
          if (knownCommunityAddress(key) == key &&
              value is Map &&
              value['removed'] is bool &&
              value['pubkey'] is String)
            key: (
              removed: value['removed'] as bool,
              pubkey: value['pubkey'] as String,
            ),
      };
    } catch (_) {
      // A record this reader cannot understand reads as empty.
      _subject = null;
      _known = {};
      _outbox = {};
    }
  }

  Future<void> _save() async {
    await (await _preferences()).setString(
      _key,
      jsonEncode({
        'subject': _subject,
        'known': _known.toList(),
        'outbox': {
          for (final MapEntry(:key, :value) in _outbox.entries)
            key: {'removed': value.removed, 'pubkey': value.pubkey},
        },
      }),
    );
  }
}

final knownCommunitiesStoreProvider = Provider<KnownCommunitiesStore>((ref) {
  final store = KnownCommunitiesStore();
  ref.onDispose(store.dispose);
  return store;
});
