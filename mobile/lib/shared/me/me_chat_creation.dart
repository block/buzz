import 'dart:convert';

import '../crypto/nip44.dart';
import 'me_sync.dart';
import 'me_journal.dart';

/// Durable, encrypted intent reused after a failed or interrupted chat creation.
class MeChatCreation {
  /// Stable channel UUID, reused for every retry.
  final String id;

  /// User-supplied name frozen before the first relay mutation.
  final String name;

  /// Selected agent public key.
  final String agent;

  /// Optional Solo project, frozen with the rest of the creation intent.
  final String? project;

  /// Create a new intent; persistence happens before [complete] performs writes.
  const MeChatCreation(this.id, this.name, this.agent, {this.project});

  /// Account and community boundary for durable creation storage.
  static String storageKey(MeSync sync) =>
      'me-chat-creation:${sync.community}:${sync.pubkey}';

  /// Restore an unfinished intent without performing relay writes.
  static MeChatCreation? pending(MeSync sync) {
    final saved = sync.prefs?.getString(storageKey(sync));
    if (saved == null) return null;
    final value =
        jsonDecode(nip44Decrypt(sync.key, saved)) as Map<String, dynamic>;
    return MeChatCreation(
      value['id'] as String,
      value['name'] as String,
      value['agent'] as String,
      project: value['project'] as String?,
    );
  }

  /// Persist before any relay mutation. Do not replace another pending intent.
  Future<void> save(MeSync sync) async {
    if (sync.disposed) throw StateError('Your community changed. Try again.');
    final previous = pending(sync);
    if (previous != null &&
        (previous.id != id ||
            previous.name != name ||
            previous.agent != agent ||
            previous.project != project)) {
      throw StateError('Resume the pending chat before starting another.');
    }
    if (sync.prefs == null ||
        !await sync.prefs!.setString(
          storageKey(sync),
          nip44Encrypt(
            sync.key,
            jsonEncode({
              'id': id,
              'name': name,
              'agent': agent,
              'project': project,
            }),
          ),
        )) {
      throw StateError('Couldn’t save this chat on your phone. Try again.');
    }
    if (sync.disposed) throw StateError('Your community changed. Try again.');
  }

  /// Abandon only this reviewed intent, preserving any relay channel/messages.
  /// Partial channels remain accessible from Home (or Solo if already placed).
  Future<void> discard(MeSync sync) async {
    if (sync.disposed) throw StateError('Your community changed. Try again.');
    final current = pending(sync);
    if (current?.id != id ||
        current?.name != name ||
        current?.agent != agent ||
        current?.project != project) {
      throw StateError('The unfinished chat changed. Review it again.');
    }
    await clearMeJournal(
      sync.prefs!,
      storageKey(sync),
      sync.prefs!.getString(storageKey(sync))!,
      checkScope: () {
        if (sync.disposed) {
          throw StateError('Your community changed. Try again.');
        }
      },
    );
  }

  /// Keep the journal through every relay step, including Me placement.
  Future<T> complete<T>(
    MeSync sync, {
    required Future<T?> Function() findChannel,
    required Future<T> Function() createChannel,
    required Future<bool> Function() hasAgent,
    required Future<void> Function() addAgent,
    required Future<bool> Function() isPlaced,
    required Future<void> Function() place,
  }) async {
    await save(sync);
    final record = sync.prefs!.getString(storageKey(sync))!;
    sync.check();
    final existing = await findChannel();
    sync.check();
    final result = existing ?? await createChannel();
    sync.check();
    final member = await hasAgent();
    sync.check();
    if (!member) await addAgent();
    sync.check();
    final placed = await isPlaced();
    sync.check();
    // Placement may be confirmed while its inner journal cleanup failed.
    if (!placed || sync.prefs?.getString(sync.pendingKey) != null) {
      await place();
    }
    sync.check();
    await clearMeJournal(
      sync.prefs!,
      storageKey(sync),
      record,
      checkScope: sync.check,
    );
    return result;
  }
}
