import 'dart:convert';

import '../crypto/nip44.dart';
import 'me_groups.dart';
import 'me_sync.dart';
import 'me_journal.dart';

/// An accepted project name and stable ID, retained until confirmed or discarded.
class MeProjectCreation {
  /// Stable UUID reused across restarts and retries.
  final String id;

  /// Reviewed name retained before the first network dependency.
  final String name;

  /// Build an intent; [complete] persists it before syncing.
  const MeProjectCreation(this.id, this.name);

  /// Isolate private drafts by both account and community.
  static String storageKey(MeSync sync) =>
      'me-project-creation:${sync.community}:${sync.pubkey}';

  /// Restore without requiring connectivity.
  static MeProjectCreation? pending(MeSync sync) {
    final saved = sync.prefs?.getString(storageKey(sync));
    if (saved == null) return null;
    final value =
        jsonDecode(nip44Decrypt(sync.key, saved)) as Map<String, dynamic>;
    return MeProjectCreation(value['id'] as String, value['name'] as String);
  }

  void _checkScope(MeSync sync) {
    if (sync.disposed) {
      throw StateError('Your community changed. Open Solo again.');
    }
  }

  /// Persist before connectivity checks or relay reads, including while offline.
  Future<void> save(MeSync sync) async {
    _checkScope(sync);
    final previous = pending(sync);
    if (previous != null && (previous.id != id || previous.name != name)) {
      throw StateError(
        'Review your unfinished project before starting another.',
      );
    }
    if (sync.prefs == null ||
        !await sync.prefs!.setString(
          storageKey(sync),
          nip44Encrypt(sync.key, jsonEncode({'id': id, 'name': name})),
        )) {
      throw StateError('Couldn’t save this project on your phone. Try again.');
    }
    _checkScope(sync);
  }

  /// Compare before removing the reviewed intent. Existing projects are kept.
  Future<void> discard(MeSync sync) async {
    _checkScope(sync);
    final current = pending(sync);
    if (current?.id != id || current?.name != name) {
      throw StateError('The unfinished project changed. Review it again.');
    }
    await clearMeJournal(
      sync.prefs!,
      storageKey(sync),
      sync.prefs!.getString(storageKey(sync))!,
      checkScope: () => _checkScope(sync),
    );
  }

  /// Retry using the same ID, preserving projects already confirmed on desktop.
  Future<void> complete(MeSync sync) async {
    await save(sync);
    final record = sync.prefs!.getString(storageKey(sync))!;
    final head = await sync.head();
    final exists =
        head != null && sync.decode(head).groups.any((g) => g['id'] == id);
    if (!exists || sync.prefs?.getString(sync.pendingKey) != null) {
      await sync.update(
        (current) => current.groups.any((g) => g['id'] == id)
            ? current
            : MeGroups(
                groups: [
                  ...current.groups,
                  {'id': id, 'name': name, 'defaultTemplateId': ''},
                ],
                assignments: current.assignments,
                channels: current.channels,
              ),
      );
    }
    await clearMeJournal(
      sync.prefs!,
      storageKey(sync),
      record,
      checkScope: () => _checkScope(sync),
    );
  }
}
