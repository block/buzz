import 'dart:convert';

import 'package:flutter/foundation.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/relay/relay.dart';
import '../../shared/theme/theme_provider.dart';

const _draftsPrefsKey = 'compose_drafts_v1';
const _maxDrafts = 50;

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
  final int updatedAt; // unix seconds

  const ComposeDraft({
    required this.key,
    required this.channelId,
    required this.threadHeadId,
    required this.text,
    required this.updatedAt,
  });

  Map<String, dynamic> toJson() => {
    'key': key,
    'channel_id': channelId,
    if (threadHeadId != null) 'thread_head_id': threadHeadId,
    'text': text,
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
    return ComposeDraft(
      key: key,
      channelId: channelId,
      threadHeadId: raw['thread_head_id'] as String?,
      text: text,
      updatedAt: updatedAt is int ? updatedAt : 0,
    );
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

  @override
  List<ComposeDraft> build() {
    // Rebuild (and re-read the identity-scoped store) whenever the active
    // community or the derived account pubkey changes.
    final config = ref.watch(relayConfigProvider);
    final pubkey = ref.watch(myPubkeyProvider) ?? 'anon';
    _prefsKey = '$_draftsPrefsKey:${config.baseUrl}:$pubkey';

    final prefs = ref.read(savedPrefsProvider);
    final raw = readMigratedPref<String>(
      prefs,
      canonicalKey: _prefsKey,
      legacyKey: '$_draftsPrefsKey:${config.storedOrigin}:$pubkey',
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

  /// Persist [text] for the composer identified by [key]. Empty text removes
  /// the draft.
  void save({
    required String key,
    required String channelId,
    String? threadHeadId,
    required String text,
  }) {
    if (text.trim().isEmpty) {
      remove(key);
      return;
    }
    final existing = state.where((d) => d.key == key).firstOrNull;
    if (existing?.text == text) return;
    final draft = ComposeDraft(
      key: key,
      channelId: channelId,
      threadHeadId: threadHeadId,
      text: text,
      updatedAt: DateTime.now().millisecondsSinceEpoch ~/ 1000,
    );
    final next = [draft, ...state.where((d) => d.key != key)];
    _persist(next.length > _maxDrafts ? next.sublist(0, _maxDrafts) : next);
  }

  void remove(String key) {
    if (!state.any((d) => d.key == key)) return;
    _persist([...state.where((d) => d.key != key)]);
  }

  String? textFor(String key) =>
      state.where((d) => d.key == key).firstOrNull?.text;

  void _persist(List<ComposeDraft> drafts) {
    state = List.unmodifiable(drafts);
    final prefs = ref.read(savedPrefsProvider);
    prefs.setString(
      _prefsKey,
      jsonEncode([for (final d in drafts) d.toJson()]),
    );
  }
}

final composeDraftsProvider =
    NotifierProvider<ComposeDraftsNotifier, List<ComposeDraft>>(
      ComposeDraftsNotifier.new,
    );
