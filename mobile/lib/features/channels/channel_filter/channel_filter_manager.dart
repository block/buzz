import 'dart:convert';
import 'dart:math';
import 'package:flutter/foundation.dart';
import 'package:nostr/nostr.dart' as nostr;
import '../../../shared/relay/relay.dart';
import '../channel_sort/channel_sort_manager.dart';
import 'channel_filter_store.dart';

/// Whole-record LWW, with an explicit preflight and confirmed readback. Cached
/// data is display-only. No automatic publication or retry of failed edits.
class ChannelFilterManager extends ChangeNotifier {
  final String pubkey;
  final ChannelSortCrypto crypto;
  final Future<List<NostrEvent>> Function() fetch;
  final Future<void> Function(String content, int createdAt) publish;
  final void Function(ChannelFilterStore store) cache;
  final bool Function(String group)? isGroupLive;
  ChannelFilterStore store;
  bool ready = false;
  bool busy = false;
  String? error;
  bool _disposed = false;
  int _createdAt = 0;

  ChannelFilterManager({
    required this.pubkey,
    required this.crypto,
    required this.fetch,
    required this.publish,
    required this.cache,
    this.isGroupLive,
    ChannelFilterStore? cached,
  }) : store = cached ?? ChannelFilterStore.empty();
  void _notify() {
    if (!_disposed) notifyListeners();
  }

  Future<ChannelFilterStore> _read() async {
    final events = await fetch();
    if (_disposed) throw StateError('Filter session changed');
    final candidates =
        events
            .where(
              (e) =>
                  e.kind == EventKind.readState &&
                  e.pubkey == pubkey &&
                  e.getTagValue('d') == 'channel-filter' &&
                  e.createdAt <=
                      DateTime.now().millisecondsSinceEpoch ~/ 1000 + 300,
            )
            .toList()
          ..sort(
            (a, b) => b.createdAt.compareTo(a.createdAt) != 0
                ? b.createdAt.compareTo(a.createdAt)
                : a.id.compareTo(b.id),
          );
    if (candidates.isEmpty) {
      if (events.isNotEmpty) {
        throw const FormatException('No valid filter head');
      }
      _createdAt = 0;
      return ChannelFilterStore.empty();
    }
    final event = candidates.first;
    nostr.Event.fromJson(jsonEncode(event.toJson()));
    final decoded = jsonDecode(crypto.decrypt(event.content));
    if (decoded is! Map<String, dynamic>) {
      throw const FormatException('Invalid filter preference');
    }
    final result = ChannelFilterStore.parse(decoded);
    _createdAt = event.createdAt;
    return result;
  }

  void dismissError() {
    error = null;
    _notify();
  }

  Future<void> refresh() async {
    if (_disposed || busy) return;
    busy = true;
    _notify();
    try {
      store = await _read();
      ready = true;
      error = null;
      cache(store);
    } catch (_) {
      ready = false;
      error = 'Could not load filters. Retry to enable changes.';
    } finally {
      busy = false;
      _notify();
    }
  }

  Future<void> save(
    String group,
    ChannelFilterMode mode, {
    int? windowSeconds,
    bool removeWindow = false,
  }) => _save(
    group,
    mode,
    windowSeconds: windowSeconds,
    removeWindow: removeWindow,
  );

  /// Best-effort cleanup after local deletion. Section IDs are never reused;
  /// failure leaves an unreachable preference, never blocks the local action.
  Future<void> clearDeletedSection(String sectionId) async {
    final group = 'section:$sectionId';
    if (store.modeFor(group) == ChannelFilterMode.all &&
        !(store.toJson()['activeWindowSeconds'] as Map? ?? {}).containsKey(
          group,
        )) {
      return;
    }
    await _save(
      group,
      ChannelFilterMode.all,
      removeWindow: true,
      deletedSection: true,
    );
  }

  Future<void> _save(
    String group,
    ChannelFilterMode mode, {
    int? windowSeconds,
    bool removeWindow = false,
    bool deletedSection = false,
  }) async {
    if (_disposed || !ready || busy) throw StateError('Filters are read-only');
    if (!deletedSection && isGroupLive?.call(group) == false) {
      throw StateError('Section no longer exists');
    }
    busy = true;
    error = null;
    final previous = store;
    try {
      store = store.edit(
        group,
        mode,
        windowSeconds: windowSeconds,
        removeWindow: removeWindow,
        remoteCreatedAt: _createdAt,
      );
      _notify();
      final fresh = await _read();
      if (!deletedSection && isGroupLive?.call(group) == false) {
        throw StateError('Section no longer exists');
      }
      final next = fresh.edit(
        group,
        mode,
        windowSeconds: windowSeconds,
        removeWindow: removeWindow,
        remoteCreatedAt: _createdAt,
      );
      final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
      final createdAt = max(now, _createdAt + 1);
      if (createdAt > now + 300) throw StateError('Preference clock is ahead');
      await publish(crypto.encrypt(jsonEncode(next.toJson())), createdAt);
      if (_disposed) throw StateError('Filter session changed');
      final confirmed = await _read();
      if (confirmed.modeFor(group) != mode ||
          confirmed.windowFor(group) != next.windowFor(group) ||
          (removeWindow &&
              (confirmed.toJson()['activeWindowSeconds'] as Map?)?.containsKey(
                    group,
                  ) ==
                  true)) {
        throw StateError('Filter save not confirmed');
      }
      store = confirmed;
      cache(store);
    } catch (_) {
      store = previous;
      error = 'Could not confirm the filter save.';
      rethrow;
    } finally {
      busy = false;
      _notify();
    }
  }

  @override
  void dispose() {
    _disposed = true;
    super.dispose();
  }
}
