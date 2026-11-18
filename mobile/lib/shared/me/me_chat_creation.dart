import 'dart:convert';

import '../crypto/nip44.dart';
import '../crypto/event_signing.dart';
import '../relay/nostr_models.dart';
import 'me_sync.dart';
import 'me_journal.dart';

/// Durable, encrypted intent reused after a failed or interrupted chat creation.
class MeChatCreation {
  /// Stable channel UUID, reused for every retry.
  final String id;

  /// Initial name frozen before the first relay mutation.
  final String name;

  /// Selected agent public key.
  final String agent;

  /// Optional Solo project, frozen with the rest of the creation intent.
  final String? project;

  /// Signed first message, reused verbatim after interrupted delivery.
  final NostrEvent? initialMessage;

  /// Create a new intent; persistence happens before [complete] performs writes.
  const MeChatCreation(
    this.id,
    this.name,
    this.agent, {
    this.project,
    this.initialMessage,
  });

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
      initialMessage: value['initialMessage'] == null
          ? null
          : NostrEvent.fromJson(
              value['initialMessage'] as Map<String, dynamic>,
            ),
    );
  }

  /// Persist before any relay mutation. Do not replace another pending intent.
  Future<void> save(MeSync sync) => _save(sync);

  Future<void> _save(MeSync sync, {String? replacingMessageId}) async {
    if (sync.disposed) throw StateError('Your community changed. Try again.');
    final previous = pending(sync);
    if (previous != null &&
        (previous.id != id ||
            previous.name != name ||
            previous.agent != agent ||
            previous.project != project ||
            (previous.initialMessage?.id != initialMessage?.id &&
                (replacingMessageId == null ||
                    previous.initialMessage?.id != replacingMessageId)))) {
      throw StateError('Resume the pending chat before starting another.');
    }
    // A matching cached value does not prove a previous disk write succeeded.
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
              if (initialMessage != null)
                'initialMessage': initialMessage!.toJson(),
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
        current?.project != project ||
        current?.initialMessage?.id != initialMessage?.id) {
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
    var record = sync.prefs!.getString(storageKey(sync))!;
    sync.check();
    final existing = await findChannel();
    sync.check();
    final result = existing ?? await createChannel();
    sync.check();
    final status = await Future.wait([hasAgent(), isPlaced()]);
    final member = status[0];
    sync.check();
    if (!member) await addAgent();
    sync.check();
    final placed = status[1];
    sync.check();
    // Placement may be confirmed while its inner journal cleanup failed.
    if (!placed || sync.prefs?.getString(sync.pendingKey) != null) {
      await place();
    }
    sync.check();
    if (initialMessage != null) {
      record = await _deliverInitialMessage(sync, record);
      sync.check();
    }
    await clearMeJournal(
      sync.prefs!,
      storageKey(sync),
      record,
      checkScope: sync.check,
    );
    return result;
  }

  // Relay ingestion rejects events older than 15 minutes, even duplicates.
  // Confirm acceptance before replacing an expired identity; a lost OK must
  // never turn one first message into two messages.
  Future<String> _deliverInitialMessage(MeSync sync, String record) async {
    var message = initialMessage!;
    final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
    if ((message.createdAt - now).abs() >= 14 * 60) {
      final existing = await sync.session.fetchHistory(
        NostrFilter(
          kinds: const [9],
          authors: [sync.pubkey],
          ids: [message.id],
          tags: {
            '#h': [id],
          },
          limit: 1,
        ),
      );
      sync.check();
      if (existing.any(
        (event) =>
            event.id == message.id &&
            event.pubkey == sync.pubkey &&
            event.kind == 9 &&
            event.getTagValue('h') == id,
      )) {
        return record;
      }
      message = NostrEvent.fromJson(
        signNostrEvent(
          kind: message.kind,
          content: message.content,
          tags: message.tags,
          secretKey: sync.privateHex,
        ).toMap(),
      );
      await MeChatCreation(
        id,
        name,
        agent,
        project: project,
        initialMessage: message,
      )._save(sync, replacingMessageId: initialMessage!.id);
      sync.check();
      record = sync.prefs!.getString(storageKey(sync))!;
    }
    await sync.session.publish(message);
    sync.check();
    return record;
  }
}
