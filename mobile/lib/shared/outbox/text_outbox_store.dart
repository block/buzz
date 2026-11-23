import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'dart:isolate';

import 'package:flutter/services.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:path/path.dart' as p;
import 'package:path_provider/path_provider.dart';
import 'package:sqlite3/sqlite3.dart';

import '../relay/nostr_models.dart';

/// A local community and its signing identity.
typedef TextOutboxScope = ({String communityId, String pubkey});

/// The recoverable states of a queued text send.
enum TextOutboxStatus { pending, failed, unconfirmed }

/// Text and addressing settled by the composer before contacting the relay.
typedef TextOutboxDraft = ({
  String channelId,
  String? parentId,
  String? rootId,
  String text,
  String content,
  List<String> recipients,
  List<List<String>> tags,
  String? implicitSoloAgent,
  bool personal,
});

/// One durable send, identified locally before it has a signed event.
final class TextOutboxMessage {
  TextOutboxMessage._(Row row)
    : id = row['id'] as String,
      draft = (
        channelId: row['channel_id'] as String,
        parentId: row['parent_id'] as String?,
        rootId: row['root_id'] as String?,
        text: row['text'] as String,
        content: row['content'] as String,
        recipients: (jsonDecode(row['recipients'] as String) as List)
            .cast<String>(),
        tags: [
          for (final tag in jsonDecode(row['tags'] as String) as List)
            (tag as List).cast<String>(),
        ],
        implicitSoloAgent: row['solo_agent'] as String?,
        personal: row['personal'] == 1,
      ),
      status = TextOutboxStatus.values.byName(row['status'] as String),
      signedEvent = switch (row['signed_event']) {
        final String data => NostrEvent.fromJson(
          jsonDecode(data) as Map<String, dynamic>,
        ),
        _ => null,
      },
      mayHaveSent = row['may_have_sent'] == 1;

  final String id;
  final TextOutboxDraft draft;
  final TextOutboxStatus status;
  final NostrEvent? signedEvent;
  final bool mayHaveSent;
}

/// Supplies a backup-excluded directory before SQLite can write any text.
final textOutboxDirectoryProvider = FutureProvider<Directory?>((ref) async {
  if (Platform.isAndroid) {
    final support = await getApplicationSupportDirectory();
    return Directory(p.join(support.path, 'message-outbox'));
  }
  if (!Platform.isIOS) return null;
  final path = await const MethodChannel(
    'buzz/private_storage',
  ).invokeMethod<String>('messageOutboxDirectory');
  if (path == null) throw StateError('Message outbox storage unavailable.');
  return Directory(path);
});

/// The app-wide serial writer for the durable text outbox.
final textOutboxStoreProvider = Provider<TextOutboxStore>((ref) {
  final store = TextOutboxStore(
    directory: () => ref.read(textOutboxDirectoryProvider.future),
  );
  ref.onDispose(store.close);
  return store;
});

/// Stores user-generated text separately from the disposable message cache.
final class TextOutboxStore {
  TextOutboxStore({required Future<Directory?> Function() directory})
    : _directory = directory;

  final Future<Directory?> Function() _directory;
  final _changes = StreamController<int>.broadcast();
  int _revision = 0;
  Future<void> _tail = Future.value();
  final _retiring = <String>{};

  /// Fires after a successful write so readers can refresh.
  Stream<int> get changes => _changes.stream;

  /// Releases listeners after queued database operations have settled.
  void close() => unawaited(_tail.whenComplete(_changes.close));

  /// Whether community removal currently prevents new sends.
  bool isRetiring(String communityId) => _retiring.contains(communityId);

  /// Fences delivery and enqueue while the saved community is removed.
  void beginRemoval(String communityId) => _retiring.add(communityId);

  /// Releases the fence after the membership and database operations settle.
  void endRemoval(String communityId) {
    _retiring.remove(communityId);
    _changes.add(++_revision);
  }

  /// Commits the complete unsigned send in one SQLite write.
  Future<void> enqueue(
    TextOutboxScope scope,
    String id,
    TextOutboxDraft draft,
  ) => _run(
    (db) {
      if (db == null) throw StateError('Message outbox storage unavailable.');
      db.execute(
        '''
      INSERT INTO outbox (
        id, community_id, pubkey, channel_id, parent_id, root_id, text,
        content, recipients, tags, solo_agent, personal
      ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
      ON CONFLICT(id) DO NOTHING
      ''',
        [
          id,
          scope.communityId,
          scope.pubkey,
          draft.channelId,
          draft.parentId,
          draft.rootId,
          draft.text,
          draft.content,
          jsonEncode(draft.recipients),
          jsonEncode(draft.tags),
          draft.implicitSoloAgent,
          draft.personal ? 1 : 0,
        ],
      );
    },
    create: true,
    write: true,
    scope: scope,
  );

  /// Reads the scope in original commit order.
  Future<List<TextOutboxMessage>> list(TextOutboxScope scope) => _run(
    (db) => [
      for (final row
          in db?.select(
                'SELECT * FROM outbox WHERE community_id = ? AND pubkey = ? '
                'ORDER BY rowid',
                [scope.communityId, scope.pubkey],
              ) ??
              const <Row>[])
        TextOutboxMessage._(row),
    ],
  );

  /// Saves a signature while the event can still be proven unsent.
  Future<void> saveSignature(
    TextOutboxScope scope,
    String id,
    NostrEvent event,
  ) => _run(
    (db) {
      _update(db, scope, id, 'signed_event = ?', [
        jsonEncode(event.toJson()),
      ], condition: ' AND may_have_sent = 0');
    },
    write: true,
    scope: scope,
  );

  /// Commits the uncertainty marker before the session writes to the socket.
  Future<void> markAttempted(TextOutboxScope scope, String id) => _run((db) {
    _update(db, scope, id, 'may_have_sent = 1', const []);
  }, scope: scope);

  /// Persists a failed, pending, or delivery-unconfirmed status.
  Future<void> setStatus(
    TextOutboxScope scope,
    String id,
    TextOutboxStatus status,
  ) => _run(
    (db) {
      _update(db, scope, id, 'status = ?', [status.name]);
    },
    write: true,
    scope: scope,
  );

  /// Requeues a failed row without changing its signed identity.
  Future<void> retry(TextOutboxScope scope, String id) => _run(
    (db) {
      db?.execute(
        "UPDATE outbox SET status = 'pending' WHERE community_id = ? "
        "AND pubkey = ? AND id = ? AND status = 'failed'",
        [scope.communityId, scope.pubkey, id],
      );
    },
    write: true,
    scope: scope,
  );

  /// Deletes a recoverable row if the user still sees a terminal status.
  Future<void> discard(TextOutboxScope scope, String id) => _run(
    (db) {
      db?.execute(
        'DELETE FROM outbox WHERE community_id = ? AND pubkey = ? '
        "AND id = ? AND status IN ('failed', 'unconfirmed')",
        [scope.communityId, scope.pubkey, id],
      );
    },
    write: true,
    scope: scope,
  );

  /// Whether a removal confirmation should warn about saved sends.
  Future<bool> hasMessages({String? communityId}) => _run(
    (db) =>
        db?.select(
          communityId == null
              ? 'SELECT 1 FROM outbox LIMIT 1'
              : 'SELECT 1 FROM outbox WHERE community_id = ? LIMIT 1',
          [?communityId],
        ).isNotEmpty ??
        false,
  );

  /// The presence check fails conservatively if the store cannot be read.
  Future<bool> shouldWarnBeforeRemoval({String? communityId}) async {
    try {
      return await hasMessages(communityId: communityId);
    } catch (_) {
      return true;
    }
  }

  /// Removes an accepted or explicitly discarded send.
  Future<void> remove(TextOutboxScope scope, String id) => _run(
    (db) {
      db?.execute(
        'DELETE FROM outbox WHERE community_id = ? AND pubkey = ? AND id = ?',
        [scope.communityId, scope.pubkey, id],
      );
    },
    write: true,
    scope: scope,
  );

  /// Removes every send for a community whose local membership was removed.
  Future<void> removeCommunity(String communityId) => _run((db) {
    db?.execute('DELETE FROM outbox WHERE community_id = ?', [communityId]);
  }, write: true);

  /// Drops scopes absent from the persisted community list before replay.
  Future<void> removeOrphans(Set<TextOutboxScope> retained) => _run((db) {
    if (db == null) return;
    _transaction(db, () {
      for (final row in db.select(
        'SELECT DISTINCT community_id, pubkey FROM outbox',
      )) {
        final scope = (
          communityId: row['community_id'] as String,
          pubkey: row['pubkey'] as String,
        );
        if (retained.contains(scope)) continue;
        db.execute('DELETE FROM outbox WHERE community_id = ? AND pubkey = ?', [
          scope.communityId,
          scope.pubkey,
        ]);
      }
    });
  });

  Future<T> _run<T>(
    T Function(Database?) operation, {
    bool create = false,
    bool write = false,
    TextOutboxScope? scope,
  }) {
    final result = _tail.then((_) async {
      if (scope != null && isRetiring(scope.communityId)) {
        throw StateError('The community is being removed.');
      }
      final directory = await _directory();
      final path = directory == null
          ? null
          : p.join(directory.path, 'outbox.sqlite');
      final result = path == null
          ? operation(null)
          : await _runOnDisk(path, create, operation);
      if (write) _changes.add(++_revision);
      return result;
    });
    _tail = result.then<void>((_) {}, onError: (Object _, StackTrace _) {});
    return result;
  }
}

void _update(
  Database? db,
  TextOutboxScope scope,
  String id,
  String set,
  List<Object?> values, {
  String condition = '',
}) {
  db?.execute(
    'UPDATE outbox SET $set '
    'WHERE community_id = ? AND pubkey = ? AND id = ?$condition',
    [...values, scope.communityId, scope.pubkey, id],
  );
  if (db?.updatedRows != 1) throw StateError('The queued message changed.');
}

Future<T> _runOnDisk<T>(
  String path,
  bool create,
  T Function(Database?) operation,
) => Isolate.run(() {
  if (!create && !File(path).existsSync()) {
    return operation(null);
  }
  if (create) File(path).parent.createSync(recursive: true);
  final db = sqlite3.open(path);
  try {
    db.execute('PRAGMA synchronous = FULL');
    db.execute('PRAGMA secure_delete = ON');
    _migrate(db);
    return operation(db);
  } finally {
    db.close();
  }
});

void _transaction(Database db, void Function() body) {
  db.execute('BEGIN IMMEDIATE');
  try {
    body();
    db.execute('COMMIT');
  } catch (_) {
    db.execute('ROLLBACK');
    rethrow;
  }
}

void _migrate(Database db) {
  const version = 1;
  if (db.userVersion == version) return;
  if (db.userVersion != 0) {
    throw StateError('Unsupported message outbox format.');
  }
  // Future versions migrate in this same transaction. Keep signed_event
  // verbatim and leave user text intact if any migration step fails.
  _transaction(db, () {
    db.execute('''
      CREATE TABLE outbox (
        id TEXT NOT NULL UNIQUE,
        community_id TEXT NOT NULL,
        pubkey TEXT NOT NULL,
        channel_id TEXT NOT NULL,
        parent_id TEXT,
        root_id TEXT,
        text TEXT NOT NULL,
        content TEXT NOT NULL,
        recipients TEXT NOT NULL,
        tags TEXT NOT NULL,
        solo_agent TEXT,
        personal INTEGER NOT NULL,
        status TEXT NOT NULL DEFAULT 'pending',
        signed_event TEXT,
        may_have_sent INTEGER NOT NULL DEFAULT 0
      )
      ''');
    db.execute('CREATE INDEX outbox_scope ON outbox(community_id, pubkey)');
    db.userVersion = version;
  });
}
