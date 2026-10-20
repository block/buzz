import 'dart:convert';

import 'package:uuid/uuid.dart';

import '../crypto/nip44.dart';
import 'me_sync.dart';
import 'me_journal.dart';

/// Durable intent written before granting a selected agent channel access.
class MeAgentChange {
  /// Agent selected by the user.
  final String agent;
  final String _scope;
  final String _record;

  MeAgentChange._(this.agent, this._scope, this._record);

  /// Scope both the journal and active preference to this chat and identity.
  static String selectionKey(MeSync sync, String channel) =>
      'me-assistant:${sync.community}:${sync.pubkey}:$channel';

  /// Create a distinct reviewed intent, even when choosing the same agent again.
  factory MeAgentChange.create(MeSync sync, String channel, String agent) =>
      MeAgentChange._(
        agent,
        selectionKey(sync, channel),
        nip44Encrypt(
          sync.key,
          jsonEncode({'agent': agent, 'revision': const Uuid().v4()}),
        ),
      );

  /// Restore locally without needing a relay connection.
  static MeAgentChange? pending(MeSync sync, String channel) {
    final scope = selectionKey(sync, channel);
    final record = sync.prefs?.getString('$scope:pending');
    if (record == null) return null;
    final value = jsonDecode(nip44Decrypt(sync.key, record)) as Map;
    return MeAgentChange._(value['agent'] as String, scope, record);
  }

  void _check(MeSync sync, String channel) {
    if (sync.disposed || _scope != selectionKey(sync, channel)) {
      throw StateError(
        'Your account or community changed. Open the chat again.',
      );
    }
  }

  /// Save before network reads or writes; refuse to overwrite another intent.
  Future<void> save(MeSync sync, String channel) async {
    _check(sync, channel);
    final existing = sync.prefs?.getString('$_scope:pending');
    if (existing != null && existing != _record) {
      throw StateError('Resume your unfinished agent change first.');
    }
    if (sync.prefs == null ||
        !await sync.prefs!.setString('$_scope:pending', _record)) {
      throw StateError('Could not save your agent choice. Try again.');
    }
    _check(sync, channel);
  }

  /// Compare the exact reviewed record before clearing it.
  Future<void> discard(MeSync sync, String channel) async {
    _check(sync, channel);
    if (sync.prefs?.getString('$_scope:pending') != _record) {
      throw StateError('The pending agent change changed. Review it again.');
    }
    await clearMeJournal(
      sync.prefs!,
      '$_scope:pending',
      _record,
      checkScope: () => _check(sync, channel),
    );
  }

  /// Reconcile membership before committing the preference. Ambiguous results
  /// retain the journal so a restart can check membership instead of adding twice.
  Future<void> complete(
    MeSync sync,
    String channel, {
    required Future<void> Function() ensureMembership,
  }) async {
    await save(sync, channel);
    sync.check();
    await ensureMembership();
    sync.check();
    _check(sync, channel);
    if (sync.prefs?.getString('$_scope:pending') != _record) {
      throw StateError('The pending agent change changed. Review it again.');
    }
    if (!await sync.prefs!.setString(_scope, agent)) {
      throw StateError('Could not save your agent choice. Retry to finish.');
    }
    await discard(sync, channel);
  }
}
