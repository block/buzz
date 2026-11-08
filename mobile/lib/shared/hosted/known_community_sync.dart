import 'dart:async';
import 'dart:convert';

import 'package:flutter/foundation.dart';
import 'package:flutter_secure_storage/flutter_secure_storage.dart';

import '../storage/secure_storage.dart';
import '../storage/serial_queue.dart';
import 'builderlab_account.dart';
import 'known_community_address.dart';

/// Durable account-list edits. Community credentials and relay membership never
/// enter this queue. Each account keeps its own snapshot and latest URL intents.
class KnownCommunitySync extends ChangeNotifier {
  KnownCommunitySync({
    required this.account,
    required this.localRelays,
    FlutterSecureStorage? storage,
    this.retryDelay = const Duration(seconds: 2),
  }) : _storage = storage ?? accountSecureStorage {
    account.subjectChanges.addListener(_sessionChanged);
  }

  final BuilderlabAccountClient account;
  final Future<List<String>> Function() localRelays;
  final FlutterSecureStorage _storage;
  final Duration retryDelay;
  final _queue = SerialQueue();
  Timer? _retry;
  bool _disposed = false;
  bool _active = true;
  Future<void>? _work;
  bool _again = false;
  String? _blockedSubject;
  int _failures = 0;

  String _key(String subject) =>
      'buzz_known_communities_v1_${base64Url.encode(utf8.encode('${account.accountOrigin.origin}\n$subject'))}';

  Future<_SyncState> _read(String subject) async {
    final raw = await _storage.read(key: _key(subject));
    return raw == null ? _SyncState() : _SyncState.decode(raw);
  }

  Future<void> _write(String subject, _SyncState state) =>
      _storage.write(key: _key(subject), value: state.encode());

  bool _current(String subject) =>
      !_disposed && account.account?.subject == subject;

  void _sessionChanged() {
    _blockedSubject = null;
    _failures = 0;
    _retry?.cancel();
    notifyListeners();
    trigger();
  }

  /// Stop retries immediately when the app leaves the foreground.
  void setActive(bool active) {
    _active = active;
    _retry?.cancel();
    if (active) trigger();
  }

  /// Persist removal before deleting the local record. The drain sends it only
  /// once the local record is absent, so failed local deletion cannot forget it
  /// remotely. Signed-out edits never borrow another account's credentials.
  Future<void> remove(String relay) async {
    final subject = account.account?.subject;
    final url = knownCommunityAddress(relay);
    if (subject == null || url == null) return;
    await _queue.run(() async {
      final state = await _read(subject);
      if (!_current(subject)) throw AccountFailure.differentAccount;
      state.pending[url] = true;
      await _write(subject, state);
    });
    if (!_disposed) notifyListeners();
  }

  /// A successful explicit join supersedes an older queued removal.
  Future<void> added(String relay) async {
    final subject = account.account?.subject;
    final url = knownCommunityAddress(relay);
    if (subject == null || url == null) return;
    await _queue.run(() async {
      final state = await _read(subject);
      if (!_current(subject)) return;
      state.local.add(url);
      state.pending[url] = false;
      await _write(subject, state);
    });
    if (!_disposed) notifyListeners();
    trigger();
  }

  /// Hide pending removals even if the account service is temporarily offline.
  Future<Set<String>> pendingRemovals(String subject) => _queue.run(() async {
    final state = await _read(subject);
    return state.pending.entries
        .where((entry) => entry.value)
        .map((entry) => entry.key)
        .toSet();
  });

  /// Run after startup, a saved-community change, sign-in, or app resume.
  void trigger() => unawaited(synchronize());

  /// Coalesce concurrent refreshes and complete after their queued work drains.
  Future<void> synchronize() {
    if (_disposed || !_active) return Future.value();
    _retry?.cancel();
    final work = _work;
    if (work != null) {
      _again = true;
      return work;
    }
    return _work = _drain().whenComplete(() => _work = null);
  }

  Future<void> _drain() async {
    do {
      _again = false;
      await _run();
    } while (_again && !_disposed && _active);
  }

  Future<void> _run() async {
    String? attemptSubject;
    try {
      final session = account.account ?? await account.restore();
      if (session == null || !_current(session.subject) || !_active) return;
      final subject = session.subject;
      attemptSubject = subject;
      if (_blockedSubject == subject) return;
      final local = (await localRelays())
          .map(knownCommunityAddress)
          .whereType<String>()
          .toSet();
      if (!_current(subject)) return;
      await _queue.run(() async {
        final state = await _read(subject);
        if (!_current(subject)) return;
        // Backfill saved QR/address destinations on the first account sign-in,
        // then upload only additions. A server-list omission never erases keys.
        for (final url in local.difference(state.local)) {
          if (state.pending[url] != true) state.pending[url] = false;
        }
        state.local = local;
        await _write(subject, state);
      });
      while (_current(subject) && _active) {
        final pending = await _queue.run(() => _read(subject));
        final entries = pending.pending.entries.where(
          (entry) => !entry.value || !local.contains(entry.key),
        );
        if (entries.isEmpty) break;
        final op = entries.first;
        if (!_current(subject)) return;
        await account.updateKnownCommunity(
          op.key,
          removed: op.value,
          expectedSubject: subject,
        );
        if (!_current(subject)) return;
        await _queue.run(() async {
          final latest = await _read(subject);
          // An edit made while the request was in flight wins over its ack.
          if (latest.pending[op.key] == op.value) {
            latest.pending.remove(op.key);
            await _write(subject, latest);
          }
        });
        if (!_disposed) notifyListeners();
      }
      _failures = 0;
    } catch (error) {
      if (_disposed || (attemptSubject != null && !_current(attemptSubject))) {
        return;
      }
      if (error == AccountFailure.denied ||
          error == AccountFailure.rejected ||
          error == AccountFailure.invalidResponse ||
          error == AccountFailure.accountSetupRequired) {
        // Preserve intent but do not hammer a refused API until a new sign-in.
        _blockedSubject = account.account?.subject;
      } else if (_active && error != AccountFailure.signedOut) {
        final multiplier = 1 << (_failures++).clamp(0, 5);
        _retry = Timer(retryDelay * multiplier, trigger);
      }
      debugPrint('Account community sync deferred: $error');
    }
  }

  @override
  void dispose() {
    _disposed = true;
    _retry?.cancel();
    account.subjectChanges.removeListener(_sessionChanged);
    super.dispose();
  }
}

class _SyncState {
  _SyncState({Set<String>? local, Map<String, bool>? pending})
    : local = local ?? {},
      pending = pending ?? {};
  Set<String> local;
  final Map<String, bool> pending;

  factory _SyncState.decode(String raw) {
    final value = jsonDecode(raw) as Map<String, dynamic>;
    return _SyncState(
      local: (value['local'] as List).cast<String>().toSet(),
      pending: (value['pending'] as Map).cast<String, bool>(),
    );
  }

  String encode() => jsonEncode({'local': local.toList(), 'pending': pending});
}
