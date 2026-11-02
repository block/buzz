import 'dart:convert';

import '../crypto/nip44.dart';
import 'me_groups.dart';
import 'me_journal.dart';
import 'me_sync.dart';

/// Durable reviewed conversion intent. Relay conversion and private Solo filing
/// are separate writes; this record keeps cleanup reachable after either fails.
class MeChatConversion {
  final String channel;
  final String visibility;
  final String joinRole;
  final List<String> labels;
  final String _scope;
  final String _record;

  MeChatConversion._(
    this.channel,
    this.visibility,
    this.joinRole,
    this.labels,
    this._scope,
    this._record,
  );

  static String _key(MeSync sync, String channel) =>
      'me-chat-conversion:${sync.community}:${sync.pubkey}:$channel';

  factory MeChatConversion.create(
    MeSync sync, {
    required String channel,
    required String visibility,
    required String joinRole,
    required List<String> labels,
  }) => MeChatConversion._(
    channel,
    visibility,
    joinRole,
    List.unmodifiable(labels),
    _key(sync, channel),
    nip44Encrypt(
      sync.key,
      jsonEncode({
        'channel': channel,
        'visibility': visibility,
        'joinRole': joinRole,
        'labels': labels,
      }),
    ),
  );

  /// Restores the exact reviewed choice without publishing anything.
  static MeChatConversion? pending(MeSync sync, String channel) {
    final record = sync.prefs?.getString(_key(sync, channel));
    if (record == null) return null;
    final data = jsonDecode(nip44Decrypt(sync.key, record)) as Map;
    if (data['channel'] != channel ||
        !const {'open', 'private'}.contains(data['visibility']) ||
        !const {'member', 'guest'}.contains(data['joinRole'])) {
      throw const FormatException('The pending conversion is invalid.');
    }
    return MeChatConversion._(
      channel,
      data['visibility'] as String,
      data['joinRole'] as String,
      List<String>.unmodifiable((data['labels'] as List).cast<String>()),
      _key(sync, channel),
      record,
    );
  }

  void _check(MeSync sync) {
    if (sync.disposed || _key(sync, channel) != _scope) {
      throw StateError(
        'Your account or community changed. Open conversion again.',
      );
    }
  }

  /// Persists before dispatch and never replaces a different unfinished choice.
  Future<void> complete(
    MeSync sync, {
    required Future<void> Function() ensureConverted,
  }) async {
    _check(sync);
    final prefs = sync.prefs;
    if (prefs == null) {
      throw StateError('Could not save conversion on this phone. Try again.');
    }
    final previous = prefs.getString(_scope);
    if (previous != null && previous != _record) {
      throw StateError('Finish the pending conversion first.');
    }
    if (!await prefs.setString(_scope, _record)) {
      throw StateError('Could not save conversion on this phone. Try again.');
    }
    _check(sync);
    await ensureConverted();
    _check(sync);
    Future<bool> stillFiled() async {
      final head = await sync.head();
      final groups = head == null ? const MeGroups() : sync.decode(head);
      return groups.channels.contains(channel) ||
          groups.assignments.containsKey(channel);
    }

    if (await stillFiled()) {
      try {
        await sync.update(
          (groups) => MeGroups(
            groups: groups.groups,
            assignments: {...groups.assignments}..remove(channel),
            channels: groups.channels.where((id) => id != channel).toList(),
          ),
        );
      } catch (_) {
        // MeSync may finish an older confirmed journal and ask its caller to
        // review again. Conversion is complete only when fresh filing agrees.
        if (await stillFiled()) rethrow;
      }
    }
    _check(sync);
    await clearMeJournal(
      prefs,
      _scope,
      _record,
      checkScope: () => _check(sync),
    );
  }
}
