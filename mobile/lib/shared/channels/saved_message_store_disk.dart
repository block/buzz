part of 'saved_message_store.dart';

const _version = 3;
// Leave space for a rollback journal containing one copy of each original
// page plus its record headers, even if the process exits during a write.
const _maxDatabasePages = 11520; // 45 MiB at 4 KiB per page.
const _maxWindowBytes = 2 * 1024 * 1024;

enum _DiskAction { read, inspect, touch, save, removeChannel, relayAuthor }

final class _DiskRequest {
  const _DiskRequest({
    required this.path,
    required this.action,
    required this.page,
    required this.databasePageLimit,
    this.events = const [],
    this.relayAuthor,
    this.payloadBudget = 0,
    this.targets,
    this.requiredEdits = const {},
  });

  final String path;
  final _DiskAction action;
  final SavedMessagePage page;
  final int databasePageLimit;
  final List<NostrEvent> events;
  final String? relayAuthor;
  final int payloadBudget;
  final Map<String, bool>? targets;
  final Set<String> requiredEdits;
}

final class _DiskResult {
  const _DiskResult({
    this.events,
    this.pruned = const {},
    this.reset = false,
    this.recencyFailure,
    this.relayAuthor,
  });

  final List<NostrEvent>? events;
  final Map<String, bool> pruned;
  final bool reset;
  final StackTrace? recencyFailure;
  final String? relayAuthor;
}

String _path(String directory, String scope) =>
    p.join(directory, '$scope-v$_version.sqlite');

Future<_DiskResult> _runDisk(_DiskRequest request) =>
    Isolate.run(() => _accessDatabase(request));

_DiskResult _accessDatabase(_DiskRequest request) {
  Database? db;
  try {
    final file = File(request.path);
    final existed = request.action == _DiskAction.removeChannel
        ? _canOpenForRemoval(file)
        : file.existsSync();
    if (!existed &&
        (request.action != _DiskAction.save || request.events.isEmpty)) {
      return const _DiskResult();
    }
    final candidates =
        request.action == _DiskAction.save &&
            request.events.length <= SavedMessageStore.maxPageEvents
        ? _cacheCandidates(request.events, request.page).toList()
        : const <NostrEvent>[];
    var knownWires = const <String, String>{};
    var pruned = const <String, bool>{};
    final requiredEdits = request.requiredEdits.toSet();
    if (existed && candidates.isNotEmpty) {
      // Recover any hot rollback journal before reading verified copies.
      db = sqlite3.open(request.path, mode: OpenMode.readWrite);
      _checkSchema(db);
      knownWires = _knownWires(db, candidates);
      if (request.page.rootId.isNotEmpty) {
        requiredEdits.addAll(_savedEditTargets(db, request.page));
        pruned = _prunedEvents(db, request.page);
        requiredEdits.addAll(pruned.keys.where((id) => pruned[id]!));
      }
      db.close();
      db = null;
    }

    // Signature verification runs with no SQLite connection open.
    final verified = _verifiedWires(
      request,
      candidates.where((event) => pruned[event.id] != false),
      knownWires,
      requiredEdits,
    );
    if (verified == null) return const _DiskResult();
    final (wires, byteSize) = verified;
    if (!existed) file.parent.createSync(recursive: true);
    db = sqlite3.open(request.path);
    db.execute('PRAGMA foreign_keys = ON');
    db.execute('PRAGMA secure_delete = ON');
    if (!existed) _initializeSchema(db);
    _checkSchema(db);
    db.execute('PRAGMA max_page_count = ${request.databasePageLimit}');
    // Avoid multiple journal segments with sector padding during a write.
    db.execute('PRAGMA cache_spill = OFF');
    return switch (request.action) {
      _DiskAction.read => _readPage(db, request.page),
      _DiskAction.inspect => _readPage(db, request.page, includePruned: true),
      _DiskAction.touch => _touchPage(db, request.page),
      _DiskAction.save => _saveWithEviction(db, request, wires, byteSize),
      _DiskAction.removeChannel => _removeChannel(db, request),
      _DiskAction.relayAuthor => _DiskResult(
        relayAuthor:
            db.select('SELECT pubkey FROM relay_author').firstOrNull?['pubkey']
                as String?,
      ),
    };
  } on SqliteException catch (error) {
    if (!_isInvalidDatabase(error)) rethrow;
    db?.close();
    db = null;
    _deleteDatabase(request.path);
    return const _DiskResult(reset: true);
  } on FormatException {
    db?.close();
    db = null;
    _deleteDatabase(request.path);
    return const _DiskResult(reset: true);
  } finally {
    db?.close();
  }
}

// A false exists() result can also mean an inaccessible path. A purge must
// distinguish that from a database that was never created.
bool _canOpenForRemoval(File file) {
  try {
    file.openSync().closeSync();
    return true;
  } on FileSystemException catch (error) {
    if (error.osError?.errorCode == _noSuchFile) return false;
    rethrow;
  }
}

bool _isInvalidDatabase(SqliteException error) =>
    error.resultCode == SqlError.SQLITE_CORRUPT ||
    error.resultCode == SqlError.SQLITE_NOTADB;

void _initializeSchema(Database db) {
  db.execute('PRAGMA page_size = 4096');
  db.execute('PRAGMA journal_mode = DELETE');
  db.execute('''
    BEGIN;
    CREATE TABLE relay_author (pubkey TEXT NOT NULL);
    CREATE TABLE pages (
      channel_id TEXT NOT NULL,
      root_id TEXT NOT NULL,
      opened_at INTEGER NOT NULL,
      byte_size INTEGER NOT NULL,
      PRIMARY KEY (channel_id, root_id)
    );
    CREATE TABLE events (id TEXT PRIMARY KEY, wire TEXT NOT NULL);
    CREATE TABLE page_events (
      channel_id TEXT NOT NULL,
      root_id TEXT NOT NULL,
      position INTEGER NOT NULL,
      event_id TEXT NOT NULL REFERENCES events(id),
      PRIMARY KEY (channel_id, root_id, position),
      FOREIGN KEY (channel_id, root_id)
        REFERENCES pages(channel_id, root_id) ON DELETE CASCADE
    );
    CREATE INDEX page_events_event_id ON page_events(event_id);
    PRAGMA user_version = $_version;
    COMMIT;
  ''');
}

void _checkSchema(Database db) {
  final version = db.select('PRAGMA user_version').single.values.single;
  if (version != _version) throw const FormatException('cache format');
}

void _ensurePrunedEvents(Database db) => db.execute('''
  CREATE TABLE IF NOT EXISTS pruned_thread_events (
    channel_id TEXT NOT NULL,
    root_id TEXT NOT NULL,
    event_id TEXT NOT NULL,
    edited INTEGER NOT NULL,
    PRIMARY KEY (channel_id, root_id, event_id),
    FOREIGN KEY (channel_id, root_id)
      REFERENCES pages(channel_id, root_id) ON DELETE CASCADE
  )
''');

bool _hasPrunedEventsTable(Database db) => db.select('''
    SELECT 1 FROM sqlite_schema
    WHERE type = 'table' AND name = 'pruned_thread_events'
  ''').isNotEmpty;

Map<String, bool> _prunedEvents(Database db, SavedMessagePage page) {
  if (!_hasPrunedEventsTable(db)) return const {};
  return {
    for (final row in db.select(
      '''
      SELECT event_id, edited FROM pruned_thread_events
      WHERE channel_id = ? AND root_id = ? ORDER BY rowid
      ''',
      [page.channelId, page.rootId],
    ))
      row['event_id'] as String: row['edited'] == 1,
  };
}

_DiskResult _readPage(
  Database db,
  SavedMessagePage page, {
  bool includePruned = false,
}) {
  final key = [page.channelId, page.rootId];
  final pruned = includePruned
      ? _prunedEvents(db, page)
      : const <String, bool>{};
  final rows = db.select('''
    SELECT wire FROM page_events JOIN events ON events.id = page_events.event_id
    WHERE channel_id = ? AND root_id = ? ORDER BY position
  ''', key);
  if (rows.isEmpty) {
    return pruned.isEmpty
        ? const _DiskResult()
        : _DiskResult(events: const [], pruned: pruned);
  }
  final events = <NostrEvent>[];
  try {
    for (final row in rows) {
      events.add(
        NostrEvent.fromJson(
          jsonDecode(row['wire'] as String) as Map<String, dynamic>,
        ),
      );
    }
  } on TypeError {
    throw const FormatException('cached event shape');
  }
  try {
    _touchPage(db, page);
  } on SqliteException catch (error, stackTrace) {
    if (_isInvalidDatabase(error)) rethrow;
    return _DiskResult(
      events: events,
      pruned: pruned,
      recencyFailure: stackTrace,
    );
  }
  return _DiskResult(events: events, pruned: pruned);
}

_DiskResult _touchPage(Database db, SavedMessagePage page) {
  db.execute(
    'UPDATE pages SET opened_at = ? WHERE channel_id = ? AND root_id = ?',
    [DateTime.now().microsecondsSinceEpoch, page.channelId, page.rootId],
  );
  return const _DiskResult();
}

Map<String, String> _knownWires(Database db, List<NostrEvent> candidates) {
  final ids = candidates.map((event) => event.id).toSet();
  return {
    for (final row in db.select(
      '''
      SELECT id, wire FROM events WHERE id IN (SELECT value FROM json_each(?))
    ''',
      [jsonEncode(ids.toList())],
    ))
      row['id'] as String: row['wire'] as String,
  };
}

Iterable<String> _savedEditTargets(Database db, SavedMessagePage page) sync* {
  for (final row in db.select(
    r'''
    SELECT wire FROM page_events JOIN events ON events.id = page_events.event_id
    WHERE channel_id = ? AND root_id = ? AND json_extract(wire, '$.kind') = ?
  ''',
    [page.channelId, page.rootId, EventKind.streamMessageEdit],
  )) {
    final event = NostrEvent.fromJson(
      jsonDecode(row['wire'] as String) as Map<String, dynamic>,
    );
    final target = _savedEditTarget(event);
    if (target != null) yield target;
  }
}

String? _savedEditTarget(NostrEvent event) => event.tags.reversed
    .where((tag) => tag.length > 1 && tag[0] == 'e')
    .firstOrNull?[1];

(Map<String, String>, int)? _verifiedWires(
  _DiskRequest request,
  Iterable<NostrEvent> candidates,
  Map<String, String> knownWires,
  Set<String> requiredEdits,
) {
  final page = request.page;
  final selected = <String, String>{};
  final rootIds = <String>{};
  final edits = <String, String>{};
  final deleted = <String>{};
  var byteSize = 0;
  for (final event in candidates) {
    if (selected.containsKey(event.id)) continue;
    if (!matchesSavedMessageAuthority(event, request.relayAuthor)) continue;
    if (event.kind == EventKind.channelThreadSummary) {
      final target = event.getTagValue('e');
      if (!rootIds.contains(target) || event.getTagValue('d') != target) {
        continue;
      }
    }
    if (EventKind.channelAuxEventKinds.contains(event.kind) &&
        !event.tags.any(
          (tag) =>
              tag.length > 1 &&
              tag[0] == 'e' &&
              (selected.containsKey(tag[1]) || tag[1] == page.rootId),
        )) {
      continue;
    }
    final wire = jsonEncode(event.toJson());
    final length = utf8.encode(wire).length;
    if (knownWires[event.id] != wire &&
        !verifySavedMessageEvent(event, relayAuthor: request.relayAuthor)) {
      // Keep the previous thread page when we cannot verify a mutation.
      // Dropping it alone could restore the text it edits or deletes.
      if (page.rootId.isNotEmpty &&
          (event.kind == EventKind.streamMessageEdit ||
              event.kind == EventKind.deletion ||
              event.kind == EventKind.nip29DeleteEvent)) {
        return null;
      }
      continue;
    }
    // A truncated response can expose originals whose edits or deletions
    // occur later in the response. Drop the whole page if it cannot fit.
    if (byteSize + length > _maxWindowBytes) return (const {}, 0);
    selected[event.id] = wire;
    if (EventKind.channelTimelineContentKinds.contains(event.kind)) {
      rootIds.add(event.id);
    } else if (requiredEdits.isNotEmpty &&
        event.kind == EventKind.streamMessageEdit) {
      final target = _savedEditTarget(event);
      if (target != null) edits[event.id] = target;
    } else if (requiredEdits.isNotEmpty &&
        (event.kind == EventKind.deletion ||
            event.kind == EventKind.nip29DeleteEvent)) {
      deleted.addAll(
        event.tags
            .where((tag) => tag.length > 1 && tag[0] == 'e')
            .map((tag) => tag[1]),
      );
    }
    byteSize += length;
  }
  // Never write original text for an observed edit without verified proof.
  final confirmed = edits.entries
      .where((edit) => !deleted.contains(edit.key))
      .map((edit) => edit.value)
      .toSet();
  for (final id in requiredEdits.difference(confirmed)) {
    final removed = selected.remove(id);
    if (removed != null) byteSize -= utf8.encode(removed).length;
  }
  return (selected, byteSize);
}

_DiskResult _saveWithEviction(
  Database db,
  _DiskRequest request,
  Map<String, String> wires,
  int byteSize,
) {
  try {
    return _savePage(db, request, wires, byteSize);
  } on SqliteException catch (error) {
    if (error.resultCode != SqlError.SQLITE_FULL) rethrow;
    // Indexes and references can fill the file before the wire budget.
    // Reclaim space from the least recently opened pages and try once.
    return _savePage(
      db,
      request,
      wires,
      byteSize,
      databasePageBudget: request.databasePageLimit * 3 ~/ 4,
    );
  }
}

_DiskResult _savePage(
  Database db,
  _DiskRequest request,
  Map<String, String> selected,
  int byteSize, {
  int? databasePageBudget,
}) {
  final page = request.page;
  final oldAuthor = db
      .select('SELECT pubkey FROM relay_author')
      .firstOrNull?['pubkey'];
  final rotated =
      oldAuthor != null &&
      request.relayAuthor != null &&
      oldAuthor != request.relayAuthor;
  final pruned = !rotated && page.rootId.isNotEmpty && request.events.isNotEmpty
      ? _prunedEvents(db, page)
      : const <String, bool>{};
  db.execute('BEGIN IMMEDIATE');
  try {
    if (rotated) {
      db.execute(
        'DELETE FROM pages; DELETE FROM events; DELETE FROM relay_author;',
      );
    }
    if (request.relayAuthor != null && (oldAuthor == null || rotated)) {
      db.execute('INSERT INTO relay_author VALUES (?)', [request.relayAuthor]);
    }
    _deletePage(db, page.channelId, page.rootId);
    if ((selected.isNotEmpty || pruned.isNotEmpty) &&
        byteSize <= request.payloadBudget) {
      _evictPages(db, request.payloadBudget - byteSize, databasePageBudget);
      final insertedAt = DateTime.now().microsecondsSinceEpoch;
      db.execute('INSERT INTO pages VALUES (?, ?, ?, ?)', [
        page.channelId,
        page.rootId,
        insertedAt,
        byteSize,
      ]);
      if (pruned.isNotEmpty) {
        db.execute(
          '''
          INSERT INTO pruned_thread_events
          SELECT ?, ?, key, value FROM json_each(?)
        ''',
          [page.channelId, page.rootId, jsonEncode(pruned)],
        );
      }
      final insertEvent = db.prepare(
        'INSERT OR IGNORE INTO events VALUES (?, ?)',
      );
      final insertReference = db.prepare(
        'INSERT INTO page_events VALUES (?, ?, ?, ?)',
      );
      try {
        var position = 0;
        for (final entry in selected.entries) {
          insertEvent.execute([entry.key, entry.value]);
          insertReference.execute([
            page.channelId,
            page.rootId,
            position++,
            entry.key,
          ]);
        }
      } finally {
        insertReference.close();
        insertEvent.close();
      }
    }
    db.execute('COMMIT');
  } catch (_) {
    if (!db.autocommit) db.execute('ROLLBACK');
    rethrow;
  }
  return const _DiskResult();
}

void _evictPages(Database db, int budget, int? databasePageBudget) {
  final pages = db.select('''
    SELECT channel_id, root_id, byte_size FROM pages ORDER BY opened_at, channel_id, root_id
  ''');
  var total = pages.fold(0, (sum, page) => sum + (page['byte_size'] as int));
  for (final page in pages) {
    if (total <= budget &&
        (databasePageBudget == null ||
            _allocatedDatabasePages(db) <= databasePageBudget)) {
      break;
    }
    _deletePage(db, page['channel_id'] as String, page['root_id'] as String);
    total -= page['byte_size'] as int;
  }
}

int _allocatedDatabasePages(Database db) =>
    db.select('''
  SELECT (SELECT page_count FROM pragma_page_count)
       - (SELECT freelist_count FROM pragma_freelist_count) AS used
''').single['used']
        as int;

Iterable<NostrEvent> _cacheCandidates(
  List<NostrEvent> events,
  SavedMessagePage page,
) sync* {
  final aux = <NostrEvent>[];
  for (final event in events) {
    if (EventKind.channelAuxEventKinds.contains(event.kind)) {
      if (event.channelId == page.channelId || event.channelId == null) {
        aux.add(event);
      }
    } else if (event.channelId == page.channelId &&
        (EventKind.channelTimelineContentKinds.contains(event.kind) ||
            event.kind == EventKind.channelThreadSummary)) {
      yield event;
    }
  }
  // The channel bridge can include deletions of reactions as a second hop.
  yield* aux.where(
    (event) =>
        event.kind != EventKind.deletion &&
        event.kind != EventKind.nip29DeleteEvent,
  );
  yield* aux.where(
    (event) =>
        event.kind == EventKind.deletion ||
        event.kind == EventKind.nip29DeleteEvent,
  );
}

_DiskResult _removeChannel(
  Database db,
  _DiskRequest request, {
  bool evict = false,
}) {
  final channel = request.page.channelId;
  final targets = request.targets;
  db.execute('BEGIN IMMEDIATE');
  try {
    if (targets == null) {
      _deletePage(db, channel, null);
    } else {
      if (evict) {
        _evictPages(
          db,
          request.payloadBudget,
          request.databasePageLimit * 3 ~/ 4,
        );
      }
      final mutations = jsonEncode(targets);
      final hasPrunedEvents = _recordPrunedEvents(db, channel, mutations);
      if (_deleteEventReferences(
        db,
        'channel_id = ? AND event_id IN (SELECT key FROM json_each(?))',
        [channel, mutations],
      )) {
        db.execute(
          '''
          UPDATE pages SET byte_size = (
            SELECT COALESCE(SUM(length(CAST(wire AS BLOB))), 0)
            FROM page_events JOIN events ON events.id = page_events.event_id
            WHERE page_events.channel_id = pages.channel_id
              AND page_events.root_id = pages.root_id
          ) WHERE channel_id = ?;
        ''',
          [channel],
        );
        final keepPrunedPages = hasPrunedEvents
            ? '''
             AND NOT EXISTS (
               SELECT 1 FROM pruned_thread_events
               WHERE channel_id = pages.channel_id AND root_id = pages.root_id
             )'''
            : '';
        db.execute(
          'DELETE FROM pages WHERE channel_id = ? AND byte_size = 0 $keepPrunedPages',
          [channel],
        );
      }
    }
    db.execute('COMMIT');
  } catch (error) {
    if (!db.autocommit) db.execute('ROLLBACK');
    if (!evict &&
        targets != null &&
        error is SqliteException &&
        error.resultCode == SqlError.SQLITE_FULL) {
      return _removeChannel(db, request, evict: true);
    }
    rethrow;
  }
  return const _DiskResult();
}

bool _recordPrunedEvents(Database db, String channel, String mutations) {
  if (!_hasPrunedEventsTable(db)) {
    final threadRows = db.select(
      '''
      SELECT 1 FROM page_events pe JOIN json_each(?) t ON t.key = pe.event_id
      WHERE pe.channel_id = ? AND pe.root_id <> '' LIMIT 1
    ''',
      [mutations, channel],
    );
    if (threadRows.isEmpty) return false;
    _ensurePrunedEvents(db);
  }
  db.execute(
    '''
        INSERT OR REPLACE INTO pruned_thread_events
        SELECT channel_id, root_id, event_id, MIN(edited)
        FROM (
          SELECT pe.channel_id, pe.root_id, pe.event_id, t.value AS edited
          FROM page_events pe JOIN json_each(?) t ON t.key = pe.event_id
          WHERE pe.channel_id = ? AND pe.root_id <> ''
          UNION ALL
          SELECT pe.channel_id, pe.root_id, pe.event_id, MIN(pe.edited, t.value)
          FROM pruned_thread_events pe JOIN json_each(?) t ON t.key = pe.event_id
          WHERE pe.channel_id = ?
        ) GROUP BY channel_id, root_id, event_id
      ''',
    [mutations, channel, mutations, channel],
  );
  db.execute(
    '''
        DELETE FROM pruned_thread_events WHERE rowid IN (
          SELECT rowid FROM (
            SELECT rowid, ROW_NUMBER() OVER (PARTITION BY root_id ORDER BY rowid DESC) AS newest
            FROM pruned_thread_events WHERE channel_id = ?
          ) WHERE newest > ?
        )
      ''',
    [channel, SavedMessageStore.maxPageEvents],
  );
  return true;
}

void _deletePage(Database db, String channel, String? root) {
  final key = [channel, ?root];
  final match = root == null
      ? 'channel_id = ?'
      : 'channel_id = ? AND root_id = ?';
  _deleteEventReferences(db, match, key);
  db.execute('DELETE FROM pages WHERE $match', key);
}

bool _deleteEventReferences(Database db, String match, List<Object?> key) {
  final removed = db.select(
    'DELETE FROM page_events WHERE $match RETURNING event_id',
    key,
  );
  if (removed.isEmpty) return false;
  final deleteEvent = db.prepare('''
    DELETE FROM events WHERE id = ?
    AND NOT EXISTS (SELECT 1 FROM page_events WHERE event_id = events.id)
  ''');
  try {
    for (final row in removed) {
      deleteEvent.execute([row['event_id']]);
    }
  } finally {
    deleteEvent.close();
  }
  return true;
}

void _deleteDatabase(String path) {
  FileSystemException? failure;
  for (final suffix in ['-wal', '-shm', '-journal']) {
    try {
      _deleteIfPresent('$path$suffix');
    } on FileSystemException catch (error) {
      failure ??= error;
    }
  }
  // Keep the database in place if any sidecar could still replay its pages.
  if (failure != null) throw failure;
  _deleteIfPresent(path);
}

void _deleteIfPresent(String path) {
  try {
    File(path).deleteSync();
  } on FileSystemException catch (error) {
    if (error.osError?.errorCode != _noSuchFile) rethrow;
  }
}

Future<bool> _sweepOnWorker(
  String directoryPath,
  Set<String>? retained,
  String? removedScope,
) => Isolate.run(() {
  final directory = Directory(directoryPath);
  final List<FileSystemEntity> entries;
  try {
    entries = directory.listSync(followLinks: false);
  } on FileSystemException catch (error) {
    if (error.osError?.errorCode == _noSuchFile) return false;
    rethrow;
  }
  final filename = RegExp(
    r'^([a-f0-9]{64})-v([0-9]+)\.sqlite(?:-wal|-shm|-journal)?$',
  );
  var obsolete = false;
  final discarded = <String>{};
  for (final entry in entries.whereType<File>()) {
    final match = filename.firstMatch(p.basename(entry.path));
    if (match == null) continue;
    final scope = match[1]!;
    final oldFormat = int.parse(match[2]!) != _version;
    if (scope == removedScope ||
        (retained != null && (!retained.contains(scope) || oldFormat))) {
      discarded.add(p.join(directoryPath, '$scope-v${match[2]}.sqlite'));
      obsolete |= oldFormat;
    }
  }
  FileSystemException? failure;
  for (final path in discarded) {
    try {
      _deleteDatabase(path);
    } on FileSystemException catch (error) {
      failure ??= error;
    }
  }
  if (failure != null) throw failure;
  return obsolete;
});
