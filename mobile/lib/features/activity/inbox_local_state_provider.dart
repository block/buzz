import 'dart:convert';

import 'package:flutter/foundation.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/relay/relay.dart';
import '../../shared/theme/community_theme_preference.dart';
import '../../shared/theme/theme_provider.dart';

const _localStateKeyPrefix = 'activity_inbox_local_state_v1';
const _maxTrackedIds = 500;

/// Local, per-device inbox row overrides — mirrors desktop's
/// `useFeedItemState`:
/// - [doneIds] marks channel-less rows (e.g. approvals without an `h` tag)
///   as read, since they have no NIP-RS context to advance.
/// - [unreadBaselines] is the per-row "mark unread" override that reopens a
///   row without rewinding the shared channel/thread read marker. It stays
///   on this device. Each maps to the newest marker covering the row when it
///   was set; the override ends once any device reads past that.
@immutable
class InboxLocalState {
  final Set<String> doneIds;
  final Map<String, int> unreadBaselines;

  /// The catch-up scope covering each override: its channel id, or
  /// `thread:<root>` for a thread row. Overrides saved before scopes have none.
  final Map<String, String> unreadScopes;

  const InboxLocalState({
    required this.doneIds,
    required this.unreadBaselines,
    this.unreadScopes = const {},
  });

  const InboxLocalState.empty()
    : doneIds = const {},
      unreadBaselines = const {},
      unreadScopes = const {};

  Set<String> get unreadIds => unreadBaselines.keys.toSet();
}

/// The baseline of an override saved before baselines existed: no read ends
/// it, as before.
const _legacyUnreadBaseline = 4294967295;

class InboxLocalStateNotifier extends Notifier<InboxLocalState> {
  String? _key;

  @override
  InboxLocalState build() {
    // Rows belong to one identity in one community, and each keeps its own cap.
    final relayUrl = ref.watch(relayConfigProvider).baseUrl;
    final pubkey = ref.watch(myPubkeyProvider);
    if (pubkey == null) {
      _key = null;
      return const InboxLocalState.empty();
    }
    final key = _key =
        '$_localStateKeyPrefix:$pubkey:${Uri.encodeComponent(normalizeCommunityRelayUrl(relayUrl))}';
    final prefs = ref.read(savedPrefsProvider);
    // Each community starts from the earlier device-wide entry, which stays
    // for the others. Its ids are event ids, so they only ever match rows in
    // their own community.
    final raw = prefs.getString(key) ?? prefs.getString(_localStateKeyPrefix);
    if (raw == null) return const InboxLocalState.empty();
    try {
      final decoded = jsonDecode(raw);
      if (decoded is! Map<String, dynamic>) {
        return const InboxLocalState.empty();
      }
      Set<String> readIds(Object? value) =>
          value is List ? value.whereType<String>().toSet() : const <String>{};
      final unread = decoded['unread'];
      final scopes = decoded['scopes'];
      return InboxLocalState(
        unreadScopes: scopes is Map
            ? {
                for (final MapEntry(:key, :value) in scopes.entries)
                  if (key is String && value is String) key: value,
              }
            : const {},
        doneIds: readIds(decoded['done']),
        unreadBaselines: unread is Map
            ? {
                for (final MapEntry(:key, :value) in unread.entries)
                  if (key is String && value is int) key: value,
              }
            : {for (final id in readIds(unread)) id: _legacyUnreadBaseline},
      );
    } catch (_) {
      return const InboxLocalState.empty();
    }
  }

  void markDone(String id) {
    _update(
      doneIds: {...state.doneIds, id},
      unreadBaselines: {...state.unreadBaselines}..remove(id),
    );
  }

  /// Reopen [ids]. [baseline] is the newest marker covering the row now
  /// (see [inboxItemReadAt]).
  /// [scope] is the catch-up scope that covers the row (see
  /// [InboxLocalState.unreadScopes]).
  void markUnread(
    Iterable<String> ids, {
    required int baseline,
    String? scope,
  }) {
    _update(
      doneIds: {...state.doneIds}..removeAll(ids),
      unreadBaselines: {
        ...state.unreadBaselines,
        for (final id in ids) id: baseline,
      },
      unreadScopes: {
        ...state.unreadScopes,
        if (scope != null)
          for (final id in ids) id: scope,
      },
    );
  }

  /// Ends the overrides a catch-up or explicit read of [scope] covers, even
  /// when no frontier moved.
  void clearScope(String scope) => clearUnread([
    for (final MapEntry(:key, :value) in state.unreadScopes.entries)
      if (value == scope) key,
  ]);

  void clearUnread(Iterable<String> ids) {
    if (!ids.any(state.unreadBaselines.containsKey)) return;
    _update(
      doneIds: state.doneIds,
      unreadBaselines: {...state.unreadBaselines}
        ..removeWhere((id, _) => ids.contains(id)),
    );
  }

  void _update({
    required Set<String> doneIds,
    required Map<String, int> unreadBaselines,
    Map<String, String>? unreadScopes,
  }) {
    Set<String> capped(Set<String> ids) => ids.length <= _maxTrackedIds
        ? ids
        : ids.skip(ids.length - _maxTrackedIds).toSet();
    final keptUnread = capped(unreadBaselines.keys.toSet());
    state = InboxLocalState(
      doneIds: capped(doneIds),
      unreadBaselines: {for (final id in keptUnread) id: unreadBaselines[id]!},
      unreadScopes: {
        for (final id in keptUnread)
          id: ?(unreadScopes ?? state.unreadScopes)[id],
      },
    );
    final key = _key;
    if (key == null) return;
    ref
        .read(savedPrefsProvider)
        .setString(
          key,
          jsonEncode({
            'done': state.doneIds.toList(),
            'unread': state.unreadBaselines,
            'scopes': state.unreadScopes,
          }),
        );
  }
}

final inboxLocalStateProvider =
    NotifierProvider<InboxLocalStateNotifier, InboxLocalState>(
      InboxLocalStateNotifier.new,
    );
