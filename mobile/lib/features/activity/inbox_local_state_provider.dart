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

  const InboxLocalState({required this.doneIds, required this.unreadBaselines});

  const InboxLocalState.empty()
    : doneIds = const {},
      unreadBaselines = const {};

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
      return InboxLocalState(
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
  void markUnread(Iterable<String> ids, {required int baseline}) {
    _update(
      doneIds: {...state.doneIds}..removeAll(ids),
      unreadBaselines: {
        ...state.unreadBaselines,
        for (final id in ids) id: baseline,
      },
    );
  }

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
  }) {
    Set<String> capped(Set<String> ids) => ids.length <= _maxTrackedIds
        ? ids
        : ids.skip(ids.length - _maxTrackedIds).toSet();
    final keptUnread = capped(unreadBaselines.keys.toSet());
    state = InboxLocalState(
      doneIds: capped(doneIds),
      unreadBaselines: {for (final id in keptUnread) id: unreadBaselines[id]!},
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
          }),
        );
  }
}

final inboxLocalStateProvider =
    NotifierProvider<InboxLocalStateNotifier, InboxLocalState>(
      InboxLocalStateNotifier.new,
    );
