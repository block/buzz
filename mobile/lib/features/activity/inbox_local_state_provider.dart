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
/// - [unreadIds] is the per-row "mark unread" override that reopens a row
///   without rewinding the shared channel/thread read marker.
@immutable
class InboxLocalState {
  final Set<String> doneIds;
  final Set<String> unreadIds;

  const InboxLocalState({required this.doneIds, required this.unreadIds});

  const InboxLocalState.empty() : doneIds = const {}, unreadIds = const {};
}

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
    var raw = prefs.getString(key);
    // The first community to open claims the earlier device-wide entry. Its
    // ids are event ids, so they only ever match rows in their own community.
    final legacy = prefs.getString(_localStateKeyPrefix);
    if (legacy != null) {
      prefs.remove(_localStateKeyPrefix);
      if (raw == null) {
        raw = legacy;
        prefs.setString(key, legacy);
      }
    }
    if (raw == null) return const InboxLocalState.empty();
    try {
      final decoded = jsonDecode(raw);
      if (decoded is! Map<String, dynamic>) {
        return const InboxLocalState.empty();
      }
      Set<String> readIds(Object? value) =>
          value is List ? value.whereType<String>().toSet() : const <String>{};
      return InboxLocalState(
        doneIds: readIds(decoded['done']),
        unreadIds: readIds(decoded['unread']),
      );
    } catch (_) {
      return const InboxLocalState.empty();
    }
  }

  void markDone(String id) {
    _update(
      doneIds: {...state.doneIds, id},
      unreadIds: {...state.unreadIds}..remove(id),
    );
  }

  void markUnread(Iterable<String> ids) {
    _update(
      doneIds: {...state.doneIds}..removeAll(ids),
      unreadIds: {...state.unreadIds, ...ids},
    );
  }

  void clearUnread(Iterable<String> ids) {
    if (!ids.any(state.unreadIds.contains)) return;
    _update(
      doneIds: state.doneIds,
      unreadIds: {...state.unreadIds}..removeAll(ids),
    );
  }

  void _update({required Set<String> doneIds, required Set<String> unreadIds}) {
    Set<String> capped(Set<String> ids) => ids.length <= _maxTrackedIds
        ? ids
        : ids.skip(ids.length - _maxTrackedIds).toSet();
    state = InboxLocalState(
      doneIds: capped(doneIds),
      unreadIds: capped(unreadIds),
    );
    final key = _key;
    if (key == null) return;
    ref
        .read(savedPrefsProvider)
        .setString(
          key,
          jsonEncode({
            'done': state.doneIds.toList(),
            'unread': state.unreadIds.toList(),
          }),
        );
  }
}

final inboxLocalStateProvider =
    NotifierProvider<InboxLocalStateNotifier, InboxLocalState>(
      InboxLocalStateNotifier.new,
    );
