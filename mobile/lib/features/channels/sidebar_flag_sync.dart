import 'dart:async';
import 'dart:convert';
import 'dart:math';

import 'package:flutter/foundation.dart';
import 'package:shared_preferences/shared_preferences.dart';

import '../../shared/crypto/event_signing.dart';
import '../../shared/read_state/read_state_time.dart';
import '../../shared/read_state/sidebar_flags.dart';
import '../../shared/relay/relay.dart';

/// Synchronizes one authoritative flag snapshot and a durable overlay of edits.
class SidebarFlagSync<T extends SidebarFlagEntry> {
  SidebarFlagSync({
    required this.pubkey,
    required this.relayUrl,
    required String coordinate,
    required SharedPreferences prefs,
    required String cacheKey,
    required Map<String, T> cachedFlags,
    required Map<String, T> Function(Map<String, dynamic>, bool strict) decode,
    required Map<String, dynamic> Function(Map<String, T>) encode,
    required T Function(bool, int) entry,
    required String Function(String) decrypt,
    required String Function(String) encrypt,
    required RelaySessionNotifier? relaySession,
    required SignedEventRelay? signedEventRelay,
    required bool remoteEnabled,
    required VoidCallback onChanged,
    required Duration startupRetryBaseDelay,
  }) : _coordinate = coordinate,
       _prefs = prefs,
       _cacheKey = cacheKey,
       _confirmed = cachedFlags,
       _decode = decode,
       _encode = encode,
       _entry = entry,
       _decrypt = decrypt,
       _encrypt = encrypt,
       _relay = relaySession,
       _signer = signedEventRelay,
       _remoteEnabled = remoteEnabled,
       _onChanged = onChanged,
       _startupRetryBaseDelay = startupRetryBaseDelay {
    _loadCacheOverlays();
  }

  final String pubkey;
  final String relayUrl;
  final String _coordinate;
  final String _cacheKey;
  final SharedPreferences _prefs;
  final Map<String, T> Function(Map<String, dynamic>, bool) _decode;
  final Map<String, dynamic> Function(Map<String, T>) _encode;
  final T Function(bool, int) _entry;
  final String Function(String) _decrypt;
  final String Function(String) _encrypt;
  final RelaySessionNotifier? _relay;
  final SignedEventRelay? _signer;
  final bool _remoteEnabled;
  final VoidCallback _onChanged;
  final Duration _startupRetryBaseDelay;
  Map<String, T> _confirmed;
  final _pending = <String, T>{};
  final _keptClears = <String, T>{};
  final _promotedClears = <String>{};
  bool _legacyCache = false;
  NostrEvent? _head;
  bool _headNeedsWrite = false;
  Timer? _writeTimer;
  Timer? _startupTimer;
  void Function()? _unsubscribe;
  static const _maxWriteFailures = 5;
  int _writeFailures = 0;
  int _startupAttempts = 0;
  bool _headLoaded = false;
  bool _liveSettled = false;
  bool _writing = false;
  bool _disposed = false;

  /// The local view includes unsent changes across reconnects and restarts.
  Map<String, T> get values => {..._confirmed, ..._pending};

  @protected
  ({NostrEvent head, Iterable<MapEntry<String, T>> entries})?
  get confirmedSnapshot {
    final head = _head;
    return head == null ? null : (head: head, entries: _confirmed.entries);
  }

  bool get _active =>
      !_disposed && (_relay?.relayUrl == null || _relay?.relayUrl == relayUrl);

  NostrFilter _filter({bool writer = false}) => NostrFilter(
    kinds: const [EventKind.readState],
    authors: [pubkey],
    tags: {
      '#d': [_coordinate],
    },
    limit: 1,
    extensions: writer ? const {'consistency': 'strong'} : const {},
  );

  void _loadCacheOverlays() {
    try {
      final raw = _prefs.getString(_cacheKey);
      if (raw == null) return;
      final cached = jsonDecode(raw) as Map<String, dynamic>;
      _legacyCache =
          _confirmed.isNotEmpty &&
          (!cached.containsKey('pending') || cached['legacyCache'] == true);
      _pending.addAll(
        _decode({'version': 1, 'channels': cached['pending']}, false),
      );
      _keptClears.addAll(
        _decode({'version': 1, 'channels': cached['keptClears']}, false)
          ..removeWhere((_, entry) => entry.enabled),
      );
      if (cached['promotedClears'] case List ids) {
        _promotedClears.addAll(
          ids.whereType<String>().where((id) => _pending[id]?.enabled == false),
        );
      }
    } catch (error) {
      debugPrint('[$_coordinate] cannot load cached flag overlays: $error');
    }
  }

  Future<bool> _persist() async {
    if (!_active) return false;
    try {
      return await _prefs.setString(
        _cacheKey,
        jsonEncode({
          ..._encode(_confirmed),
          'pending': _encode(_pending)['channels'],
          if (_keptClears.isNotEmpty)
            'keptClears': _encode(_keptClears)['channels'],
          if (_promotedClears.isNotEmpty)
            'promotedClears': _promotedClears.toList(),
          if (_legacyCache) 'legacyCache': true,
        }),
      );
    } catch (error) {
      debugPrint('[$_coordinate] cannot save flag state: $error');
      return false;
    }
  }

  /// Start the read lanes and resume any journaled edits.
  Future<void> initialize() async {
    if (!_remoteEnabled || !_active || _relay == null) return;
    await _startReadLanes();
    _scheduleWrite();
  }

  Future<void> _startReadLanes() async {
    if (!_active) return;
    if (!_headLoaded) _headLoaded = await _recoverHead();
    if (!_active) return;
    if (!_liveSettled) {
      try {
        final unsubscribe = await _relay!.subscribe(
          _filter(),
          (event) => unawaited(_acceptRead(event)),
          onClosed: (_) => _liveSettled = true,
        );
        if (!_active) {
          unsubscribe();
          return;
        }
        _unsubscribe = unsubscribe;
        _liveSettled = true;
      } catch (error) {
        debugPrint('[$_coordinate] flag subscription failed: $error');
      }
    }
    if ((!_headLoaded || !_liveSettled) && _active) {
      final delay = min(
        _startupRetryBaseDelay.inMilliseconds << min(_startupAttempts++, 5),
        30000,
      );
      _startupTimer = Timer(
        Duration(milliseconds: delay),
        () => unawaited(_startReadLanes()),
      );
    }
  }

  /// Refresh the view and give any pending save another chance on resume.
  void refreshFromRelay() {
    if (!_remoteEnabled || !_active || _relay == null) return;
    unawaited(_resume());
  }

  Future<void> _resume() async {
    if (_legacyCache || (_pending.isEmpty && !_writing)) await _recoverHead();
    if (_writeTimer?.isActive == true || _writing) return;
    _writeFailures = 0;
    _scheduleWrite();
  }

  Future<bool> _recoverHead() async {
    try {
      if (_legacyCache) return await _recoverLegacyCache();
      final events = await _relay!.fetchHistory(_filter());
      if (!_active) return false;
      if (events.isEmpty) return true;
      return await _acceptRead(events.single);
    } on FormatException catch (error) {
      // An unreadable legacy head can recover on a valid live event or resume.
      debugPrint('[$_coordinate] unreadable flag snapshot: $error');
      return _legacyCache;
    } catch (error) {
      debugPrint('[$_coordinate] flag read failed: $error');
      return false;
    }
  }

  Future<bool> _recoverLegacyCache() async {
    final (head, snapshot) = await _writerHead();
    if (!_legacyCache) return true;

    // Old caches combine confirmed entries with offline edits. At the cap,
    // an absent flag may have been evicted, so require it to postdate the
    // snapshot's second. Reconcile legacy clears against the snapshot below.
    final firstLaterMillis = head != null && snapshot.length >= maxSidebarFlags
        ? (head.createdAt + 1) * 1000
        : 0;
    for (final local in _confirmed.entries) {
      if (_pending.containsKey(local.key)) continue;
      if (!local.value.enabled) {
        _keptClears[local.key] = local.value;
        continue;
      }
      final remote = snapshot[local.key];
      final localMillis = sidebarFlagTimeMillis(local.value.updatedAt);
      final remoteMillis = remote == null
          ? firstLaterMillis
          : sidebarFlagTimeMillis(remote.updatedAt);
      // A newer enable still matters when the current values agree. Its
      // timestamp protects it if a delayed clear arrives before the write.
      if (localMillis > remoteMillis ||
          (localMillis == remoteMillis && remote?.enabled != true)) {
        _pending[local.key] = local.value;
      }
    }
    _head = head;
    _headNeedsWrite = head != null;
    _replaceConfirmed(snapshot);
    _legacyCache = false;
    if (!await _persist() || !_active) return false;
    if (identical(_head, head)) _headNeedsWrite = false;
    _onChanged();
    _scheduleWrite();
    return true;
  }

  void _replaceConfirmed(Map<String, T> snapshot) {
    _confirmed = snapshot;
    // These conditional clears may only replace an existing enable. If
    // another writer removes it or restores an older clear, park the edit.
    _promotedClears.removeWhere((id) {
      final clear = _pending[id]!;
      final remote = snapshot[id];
      if (remote != null &&
          (remote.enabled ||
              sidebarFlagTimeMillis(remote.updatedAt) >=
                  sidebarFlagTimeMillis(clear.updatedAt))) {
        return false;
      }
      _keptClears[id] = clear;
      _pending.remove(id);
      return true;
    });
    _keptClears.removeWhere((id, clear) {
      final remote = snapshot[id];
      if (remote == null) return false;

      final clearMillis = sidebarFlagTimeMillis(clear.updatedAt);
      final remoteMillis = sidebarFlagTimeMillis(remote.updatedAt);
      if (!remote.enabled && remoteMillis < clearMillis) return false;
      if (remote.enabled &&
          remoteMillis <= clearMillis &&
          !_pending.containsKey(id)) {
        // A readback can only retire the promotion its write captured.
        _pending[id] = _entry(clear.enabled, clear.updatedAt);
        _promotedClears.add(id);
      }
      return true;
    });
  }

  bool _matches(NostrEvent event) =>
      event.kind == EventKind.readState &&
      event.pubkey == pubkey &&
      event.getTagValue('d') == _coordinate;

  Map<String, T> _decodeEvent(NostrEvent event) => _decode(
    jsonDecode(_decrypt(event.content)) as Map<String, dynamic>,
    true,
  );

  Future<bool> _acceptRead(NostrEvent event) async {
    if (!_active || !_matches(event)) return false;
    final head = _head;
    if (head != null && !_isNewer(event, head)) {
      if (!_headNeedsWrite || head.id != event.id) return true;
    }
    final Map<String, T> confirmed;
    try {
      confirmed = _decodeEvent(event);
    } catch (error) {
      debugPrint('[$_coordinate] unreadable flag snapshot: $error');
      return true;
    }
    if (_legacyCache) return await _recoverHead();
    _head = event;
    _headNeedsWrite = true;
    _replaceConfirmed(confirmed);
    if (!await _persist() || !_active) return false;
    if (identical(_head, event)) _headNeedsWrite = false;
    _onChanged();
    if (_writeFailures >= _maxWriteFailures) {
      _writeFailures = 0;
    }
    _scheduleWrite();
    return true;
  }

  bool _isNewer(NostrEvent candidate, NostrEvent head) =>
      candidate.createdAt > head.createdAt ||
      (candidate.createdAt == head.createdAt &&
          candidate.id.compareTo(head.id) < 0);

  /// Keep the pending change locally until the writer confirms it.
  void setFlag(String id, bool enabled, {bool skipAbsentClear = false}) {
    if (!_active) return;
    final previous = _pending[id] ?? _confirmed[id];
    if ((previous?.enabled == enabled && !_promotedClears.contains(id)) ||
        (skipAbsentClear && !enabled && previous == null)) {
      return;
    }
    final keptClear = _keptClears.remove(id);
    _promotedClears.remove(id);
    _pending[id] = _entry(
      enabled,
      _nextTimestamp(_pending[id] ?? keptClear, _confirmed[id]),
    );
    // Invoke the prefs setter in this turn so a successor manager created
    // during a disconnect can see the journal immediately.
    unawaited(_persist());
    _onChanged();
    _writeFailures = 0;
    _writeTimer?.cancel();
    _scheduleWrite();
  }

  int _nextTimestamp(T? pending, T? confirmed) => max(
    DateTime.now().millisecondsSinceEpoch,
    max(
          sidebarFlagTimeMillis(pending?.updatedAt ?? 0),
          sidebarFlagTimeMillis(confirmed?.updatedAt ?? 0),
        ) +
        1,
  );

  void _scheduleWrite({Duration delay = const Duration(seconds: 5)}) {
    if (!_remoteEnabled ||
        !_active ||
        _legacyCache ||
        _relay == null ||
        _signer == null ||
        _pending.isEmpty ||
        _writing ||
        _writeTimer?.isActive == true) {
      return;
    }
    _writeTimer = Timer(delay, () {
      _writeTimer = null;
      unawaited(_writePending());
    });
  }

  Future<(NostrEvent?, Map<String, T>)> _writerHead() async {
    final events = await _relay!.queryRelay([_filter(writer: true)]);
    if (!_active) throw StateError('Flag query belongs to a retired session');
    if (events.isEmpty) return (null, <String, T>{});
    final event = events.single;
    if (!_matches(event)) throw const FormatException('Unexpected flag record');
    try {
      verifiedNostrEventFromJson(jsonEncode(event.toJson()));
      return (event, _decodeEvent(event));
    } catch (_) {
      throw const FormatException('Unreadable writer flag record');
    }
  }

  Future<void> _writePending() async {
    if (!_active || _pending.isEmpty) return;
    _writing = true;
    final headAtStart = _head;
    try {
      final (head, snapshot) = await _writerHead();
      final observed = _head;
      if (observed != null && (head == null || _isNewer(observed, head))) {
        throw StateError(
          'Writer flag snapshot is older than the observed head',
        );
      }
      _head = head;
      _replaceConfirmed(snapshot);
      final edits = _nextWriteBatch();
      final settled = <String, T>{};
      final outgoing = Map.of(snapshot);
      for (final edit in edits.entries) {
        final latest = snapshot[edit.key];
        var value = edit.value;
        if (latest != null) {
          final editMillis = sidebarFlagTimeMillis(value.updatedAt);
          final age = sidebarFlagTimeMillis(
            latest.updatedAt,
          ).compareTo(editMillis);
          if (age > 0 || (age == 0 && latest.enabled == edit.value.enabled)) {
            settled[edit.key] = edit.value;
            continue;
          }
          // Break a conflicting tie for older clients that keep their local
          // value at equal timestamps. Strictly newer writer values win.
          if (age == 0) {
            value = _entry(value.enabled, editMillis + 1);
          } else if (latest.updatedAt > value.updatedAt) {
            // Keep recovered seconds stamps unless the writer already uses
            // milliseconds for this flag and would win a raw comparison.
            value = _entry(value.enabled, editMillis);
          }
        }
        outgoing[edit.key] = value;
      }
      if (settled.isNotEmpty) {
        await _retireEdits(settled);
        edits.removeWhere((id, _) => settled.containsKey(id));
      } else {
        if (!await _persist()) throw StateError('Flag edit is not journaled');
        if (_active && observed?.id != head?.id) _onChanged();
      }
      if (!_active) return;
      if (edits.isEmpty) {
        _writeFailures = 0;
        return;
      }
      if (_head?.id != head?.id) {
        throw StateError('Flag head changed before publish');
      }
      boundSidebarFlags(outgoing, edits.keys.toSet());
      if (!edits.keys.every(outgoing.containsKey)) {
        throw StateError('Too many pending edits for one flag record');
      }
      final result = await _signer!.submit(
        kind: EventKind.readState,
        content: _encrypt(jsonEncode(_encode(outgoing))),
        tags: [
          ['d', _coordinate],
          ['t', _coordinate],
        ],
        createdAt: max(currentUnixSeconds(), (head?.createdAt ?? 0) + 1),
        // The relay can wait for its rate limiter before calling this.
        mayPublish: () async => _active && _head?.id == head?.id,
      );
      final (retained, confirmed) = await _writerHead();
      if (retained != null && (_head == null || !_isNewer(_head!, retained))) {
        _head = retained;
        _replaceConfirmed(confirmed);
      }
      // A live snapshot can supersede the publish while readback is in flight.
      if (retained?.id != result.id || _head?.id != result.id) {
        await _persist();
        if (_active) _onChanged();
        throw StateError('Flag write was superseded');
      }
      await _retireEdits(edits);
      _writeFailures = 0;
    } on FormatException catch (error) {
      // A malformed head needs a new valid snapshot or a resume, not polling.
      _writeFailures = identical(_head, headAtStart) ? _maxWriteFailures : 1;
      debugPrint(
        '[$_coordinate] flag publish waiting for a valid head: $error',
      );
    } catch (error) {
      _writeFailures++;
      debugPrint('[$_coordinate] flag publish pending: $error');
    } finally {
      _writing = false;
      // Reconnect, resume or another edit also re-arms the bounded retry.
      if (_writeFailures < _maxWriteFailures) {
        _scheduleWrite(delay: Duration(seconds: 2 << _writeFailures));
      }
    }
  }

  Map<String, T> _nextWriteBatch() {
    if (_pending.length <= maxSidebarFlags) return Map.of(_pending);

    // Older edits go first, so subsequent writes can retain newer flags
    // when the complete journal exceeds the writer's record limit.
    final edits = _pending.entries.toList()
      ..sort((left, right) {
        final byTime = sidebarFlagTimeMillis(
          left.value.updatedAt,
        ).compareTo(sidebarFlagTimeMillis(right.value.updatedAt));
        return byTime == 0 ? left.key.compareTo(right.key) : byTime;
      });
    return Map.fromEntries(edits.take(maxSidebarFlags));
  }

  // Retire exact intents that the writer reflects or supersedes, leaving edits
  // that the user made during a relay read or publish pending.
  Future<void> _retireEdits(Map<String, T> settled) async {
    final retired = <String, T>{};
    final retiredPromotions = <String>{};
    for (final edit in settled.entries) {
      if (identical(_pending[edit.key], edit.value)) {
        retired[edit.key] = edit.value;
        _pending.remove(edit.key);
        if (_promotedClears.remove(edit.key)) retiredPromotions.add(edit.key);
      }
    }
    if (!await _persist()) {
      for (final edit in retired.entries) {
        if (_pending.containsKey(edit.key)) continue;
        _pending[edit.key] = edit.value;
        if (retiredPromotions.contains(edit.key)) _promotedClears.add(edit.key);
      }
      throw StateError('Flag settlement was not saved');
    }
    if (_active) _onChanged();
  }

  /// Retire this manager. Its successor reads any pending edits from disk.
  void dispose() {
    if (_disposed) return;
    _disposed = true;
    _startupTimer?.cancel();
    _writeTimer?.cancel();
    _unsubscribe?.call();
  }
}
