import 'dart:async';
import 'dart:convert';

import 'package:flutter/foundation.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../mentions/bound_mentions.dart';
import '../mentions/mention_bindings.dart';
import '../relay/relay.dart';
import '../theme/theme_provider.dart';
import '../utils/string_utils.dart';

const _draftsPrefsKey = 'compose_drafts_v1';
const _maxDrafts = 50;

/// A picked mention saved with its draft: the exact key bound to the literal
/// `@Label` at `[start, end)` of the text. Position, not label, identifies
/// it, so two namesakes that both read `@Sam` keep their own recipients.
/// Display only in the chooser and chip; it grants no authority.
@immutable
class ComposeDraftMention {
  final int start;
  final int end;
  final String pubkey;

  /// Set on a binding migrated from a label-keyed record, until the channel
  /// roster has been read: the old send let a longer member name claim the
  /// occurrence (`@Sam Smith` over a picked `@Sam`), and the roster is not
  /// known where drafts are read.
  final bool legacy;

  const ComposeDraftMention({
    required this.start,
    required this.end,
    required this.pubkey,
    this.legacy = false,
  });

  Map<String, dynamic> toJson() => {
    'start': start,
    'end': end,
    'pubkey': pubkey,
    if (legacy) 'legacy': true,
  };

  /// The mention [raw] describes inside [text], or null when it is not a
  /// well-formed, bound `@Label` ending at a name boundary.
  static ComposeDraftMention? fromJson(Object? raw, String text) {
    if (raw is! Map<String, dynamic>) return null;
    final start = raw['start'];
    final end = raw['end'];
    final pubkey = raw['pubkey'];
    if (start is! int ||
        end is! int ||
        pubkey is! String ||
        !isHexPubkey(pubkey) ||
        start < 0 ||
        end <= start + 1 ||
        end > text.length ||
        text.codeUnitAt(start) != 0x40 /* @ */ ||
        !mentionEndsAtBoundary(text, end)) {
      return null;
    }
    return ComposeDraftMention(
      start: start,
      end: end,
      pubkey: pubkey.toLowerCase(),
      legacy: raw['legacy'] == true,
    );
  }

  @override
  bool operator ==(Object other) =>
      other is ComposeDraftMention &&
      other.start == start &&
      other.end == end &&
      other.pubkey == pubkey &&
      other.legacy == legacy;

  @override
  int get hashCode => Object.hash(start, end, pubkey, legacy);

  @override
  String toString() =>
      'ComposeDraftMention($start, $end, $pubkey${legacy ? ', legacy' : ''})';
}

/// A locally persisted, unsent composer draft.
///
/// Mobile has no durable relay-backed draft store (desktop keeps drafts
/// locally too), so drafts are device-local by design: the inbox Drafts
/// filter reflects what this device's composer has in flight.
@immutable
class ComposeDraft {
  /// `<channelId>` for channel composers, `<channelId>:<threadHeadId>` for
  /// thread composers.
  final String key;
  final String channelId;
  final String? threadHeadId;
  final String text;

  /// Picked mentions bound to exact keys at their place in [text], in text
  /// order and never overlapping. Never authorization metadata.
  final List<ComposeDraftMention> mentions;

  /// Set when the saved bindings could not be read. Send must refuse while
  /// the text still holds an `@`, since nothing says who it was for.
  final bool mentionsInvalid;

  /// Recording whose transcript is included in this same persisted snapshot.
  final String? transcriptionId;

  final int updatedAt; // unix seconds

  const ComposeDraft({
    required this.key,
    required this.channelId,
    required this.threadHeadId,
    required this.text,
    required this.updatedAt,
    this.mentions = const [],
    this.mentionsInvalid = false,
    this.transcriptionId,
  });

  Map<String, dynamic> toJson() => {
    'key': key,
    'channel_id': channelId,
    if (threadHeadId != null) 'thread_head_id': threadHeadId,
    'text': text,
    // A null list is the unreadable-record tombstone.
    'mentions': mentionsInvalid
        ? null
        : [for (final mention in mentions) mention.toJson()],
    if (transcriptionId != null) 'transcription_id': transcriptionId,
    'updated_at': updatedAt,
  };

  static ComposeDraft? fromJson(Object? raw) {
    if (raw is! Map<String, dynamic>) return null;
    final key = raw['key'];
    final channelId = raw['channel_id'];
    final text = raw['text'];
    final updatedAt = raw['updated_at'];
    if (key is! String || channelId is! String || text is! String) return null;
    if (text.trim().isEmpty) return null;
    final (mentions, invalid) = _readMentions(raw, text);
    return ComposeDraft(
      key: key,
      channelId: channelId,
      threadHeadId: raw['thread_head_id'] is String
          ? raw['thread_head_id'] as String
          : null,
      mentions: mentions,
      mentionsInvalid: invalid,
      text: text,
      transcriptionId: raw['transcription_id'] is String
          ? raw['transcription_id'] as String
          : null,
      updatedAt: updatedAt is int ? updatedAt : 0,
    );
  }

  /// The saved bindings, or the invalid flag when they cannot be read. A
  /// record from before positions were saved holds `mention_keys`, labels
  /// bound to keys. Those resolve as the old send resolved them: one
  /// longest-match pass over every saved label, so `@Scout @Scout Marie`
  /// binds Scout once and Scout Marie once. Backticks read as spaces here,
  /// as the old saver read them when it kept a label's key: `` `@Scout` ``
  /// keeps Scout's binding. The old send opened no mention after a backtick,
  /// and the composer holds to that when it takes the draft: a binding
  /// inside the code it draws is dormant, notifying nobody until the code
  /// goes, and one after a backtick that closes no code is dropped. Member
  /// names competed in the old pass too; each migrated binding is marked
  /// [ComposeDraftMention.legacy] until the composer has checked it against
  /// the roster.
  static (List<ComposeDraftMention>, bool) _readMentions(
    Map<String, dynamic> raw,
    String text,
  ) {
    final found = <ComposeDraftMention>[];
    if (raw.containsKey('mentions')) {
      final list = raw['mentions'];
      if (list is! List) return (const [], true);
      for (final item in list) {
        final mention = ComposeDraftMention.fromJson(item, text);
        if (mention == null) return (const [], true);
        found.add(mention);
      }
    } else if (raw.containsKey('mention_keys')) {
      final bindings = raw['mention_keys'];
      if (bindings is! Map<String, dynamic>) return (const [], true);
      final keys = <String, String>{};
      for (final entry in bindings.entries) {
        final value = entry.value;
        final label = entry.key.toLowerCase();
        // Two labels differing only by case never said which key was meant.
        if (label.isEmpty ||
            value is! String ||
            !isHexPubkey(value) ||
            keys.containsKey(label)) {
          return (const [], true);
        }
        keys[label] = value.toLowerCase();
      }
      for (final range in mentionOccurrences(
        text.replaceAll('`', ' '),
        keys.keys,
      )) {
        found.add(
          ComposeDraftMention(
            start: range.start,
            end: range.end,
            pubkey: keys[range.label]!,
            legacy: true,
          ),
        );
      }
    }
    found.sort((a, b) => a.start.compareTo(b.start));
    final mentions = <ComposeDraftMention>[];
    for (final mention in found) {
      if (mentions.isEmpty || mention.start >= mentions.last.end) {
        mentions.add(mention);
      }
    }
    return (List.unmodifiable(mentions), false);
  }
}

String composeDraftKey(String channelId, {String? threadHeadId}) =>
    threadHeadId == null ? channelId : '$channelId:$threadHeadId';

/// SharedPreferences-backed store of unsent composer drafts, newest first.
///
/// Drafts are plaintext, so persistence is namespaced by community (relay)
/// and account pubkey: switching community or account rebuilds this provider
/// against that identity's own store and can never surface another
/// identity's draft text.
class ComposeDraftsNotifier extends Notifier<List<ComposeDraft>> {
  late String _prefsKey;
  Future<bool> _pendingWrite = Future.value(true);

  @override
  List<ComposeDraft> build() {
    // Rebuild (and re-read the identity-scoped store) whenever the active
    // community or the derived account pubkey changes.
    final config = ref.watch(relayConfigProvider);
    final pubkey = ref.watch(myPubkeyProvider) ?? 'anon';
    _prefsKey = _storeKey(config.baseUrl, pubkey);
    _pendingWrite = Future.value(true);
    return _readStore(config, pubkey);
  }

  static String _storeKey(String origin, String pubkey) =>
      '$_draftsPrefsKey:$origin:$pubkey';

  List<ComposeDraft> _readStore(RelayConfig config, String pubkey) {
    final prefs = ref.read(savedPrefsProvider);
    final raw = readMigratedPref<String>(
      prefs,
      canonicalKey: _storeKey(config.baseUrl, pubkey),
      legacyKey: _storeKey(config.storedOrigin, pubkey),
      read: prefs.getString,
      write: prefs.setString,
    );
    if (raw == null) return const [];
    try {
      final decoded = jsonDecode(raw);
      if (decoded is! List) return const [];
      final drafts = decoded
          .map(ComposeDraft.fromJson)
          .whereType<ComposeDraft>()
          .toList();
      drafts.sort((a, b) => b.updatedAt.compareTo(a.updatedAt));
      return drafts;
    } catch (_) {
      return const [];
    }
  }

  /// Adds [text] after any draft at [key] in the store of the community at
  /// [config], which need not be the selected one. Another community's
  /// store is written without being loaded here, so its text never appears
  /// in this community. Completes once it's saved.
  Future<void> append({
    required RelayConfig config,
    required String key,
    required String channelId,
    String? threadHeadId,
    required String text,
  }) {
    final pubkey = pubkeyFromNsec(config.nsec) ?? 'anon';
    final selected = _storeKey(config.baseUrl, pubkey) == _prefsKey;
    final drafts = selected ? state : _readStore(config, pubkey);
    final existing = drafts.where((d) => d.key == key).firstOrNull;
    final existingText = existing?.text.trimRight() ?? '';
    final draft = ComposeDraft(
      key: key,
      channelId: channelId,
      threadHeadId: threadHeadId,
      text: existingText.isEmpty ? text : '$existingText\n\n$text',
      mentions: existing?.mentions ?? const [],
      mentionsInvalid: existing?.mentionsInvalid ?? false,
      transcriptionId: existing?.transcriptionId,
      updatedAt: DateTime.now().millisecondsSinceEpoch ~/ 1000,
    );
    final next = [draft, ...drafts.where((d) => d.key != key)];
    final kept = next.length > _maxDrafts ? next.sublist(0, _maxDrafts) : next;
    if (!selected) return _write(_storeKey(config.baseUrl, pubkey), kept);
    state = List.unmodifiable(kept);
    return _write(_prefsKey, kept);
  }

  /// Persist [text] for the composer identified by [key]. Empty text removes
  /// the draft.
  void save({
    required String key,
    required String channelId,
    String? threadHeadId,
    required String text,
    List<ComposeDraftMention> mentions = const [],
    bool mentionsInvalid = false,
    String? transcriptionId,
  }) {
    if (text.trim().isEmpty) {
      remove(key);
      return;
    }
    final existing = state.where((d) => d.key == key).firstOrNull;
    final insertion = transcriptionId ?? existing?.transcriptionId;
    if (existing?.text == text &&
        existing?.mentionsInvalid == mentionsInvalid &&
        existing?.transcriptionId == insertion &&
        listEquals(existing?.mentions, mentions)) {
      return;
    }
    final draft = ComposeDraft(
      key: key,
      channelId: channelId,
      threadHeadId: threadHeadId,
      text: text,
      mentions: List.unmodifiable(mentions),
      mentionsInvalid: mentionsInvalid,
      transcriptionId: insertion,
      updatedAt: DateTime.now().millisecondsSinceEpoch ~/ 1000,
    );
    final next = [draft, ...state.where((d) => d.key != key)];
    _persist(next.length > _maxDrafts ? next.sublist(0, _maxDrafts) : next);
  }

  void remove(String key) {
    if (!state.any((d) => d.key == key)) return;
    _persist([...state.where((d) => d.key != key)]);
  }

  /// Bind late delivery to the submitted store and complete draft snapshot.
  VoidCallback cleanupAfterSend(String key) {
    final scope = _prefsKey;
    final config = ref.read(relayConfigProvider);
    final pubkey = ref.read(myPubkeyProvider) ?? 'anon';
    final submitted = draftFor(key);
    return () {
      if (submitted == null || !ref.mounted) return;
      final selected = scope == _prefsKey;
      final drafts = selected ? state : _readStore(config, pubkey);
      final current = drafts.where((draft) => draft.key == key).firstOrNull;
      if (current == null ||
          jsonEncode(current.toJson()) != jsonEncode(submitted.toJson())) {
        return;
      }
      if (selected) {
        remove(key);
      } else {
        unawaited(_write(scope, drafts.where((d) => d.key != key).toList()));
      }
    };
  }

  /// Read text and selected identities from the same persisted snapshot.
  ComposeDraft? draftFor(String key) =>
      state.where((d) => d.key == key).firstOrNull;

  String? textFor(String key) => draftFor(key)?.text;

  /// Confirms the draft snapshot reached storage before dropping its source.
  Future<bool> ensurePersisted() async {
    final scope = _prefsKey;
    await _pendingWrite;
    if (_prefsKey != scope) return false;
    // A rebuilt provider may have read a cache entry from a failed platform
    // write. Confirm the current snapshot durably before releasing its audio.
    return _pendingWrite = _writeDrafts(state);
  }

  void _persist(List<ComposeDraft> drafts) {
    state = List.unmodifiable(drafts);
    _pendingWrite = _writeDrafts(drafts);
  }

  Future<bool> _writeDrafts(List<ComposeDraft> drafts) async {
    final prefs = ref.read(savedPrefsProvider);
    try {
      return await prefs.setString(
        _prefsKey,
        jsonEncode([for (final d in drafts) d.toJson()]),
      );
    } catch (_) {
      return false;
    }
  }

  Future<void> _write(String prefsKey, List<ComposeDraft> drafts) => ref
      .read(savedPrefsProvider)
      .setString(prefsKey, jsonEncode([for (final d in drafts) d.toJson()]));
}

final composeDraftsProvider =
    NotifierProvider<ComposeDraftsNotifier, List<ComposeDraft>>(
      ComposeDraftsNotifier.new,
    );
