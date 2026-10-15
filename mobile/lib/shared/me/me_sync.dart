import 'dart:async';
import 'dart:convert';

import 'package:flutter/widgets.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:shared_preferences/shared_preferences.dart';
import '../theme/theme_provider.dart';

import '../crypto/nip44.dart';
import '../relay/relay.dart';
import 'me_groups.dart';
import 'me_recovery.dart';

/// A live, account/community-scoped reader for desktop's encrypted Solo record.
final meGroupsProvider = StreamProvider<MeGroups>((ref) {
  final service = ref.watch(meSyncProvider);
  final controller = StreamController<MeGroups>();
  var disposed = false;
  void Function()? unsubscribe;
  NostrEvent? latest;
  var hasSnapshot = false;
  Future<void> accept(NostrEvent event) async {
    if (disposed ||
        (latest != null &&
            event.id != latest!.id &&
            !meEventIsNewer(event, latest!))) {
      return;
    }
    try {
      final value = service.decode(event);
      await service.cache(event);
      if (disposed ||
          (latest != null &&
              event.id != latest!.id &&
              !meEventIsNewer(event, latest!))) {
        return;
      }
      latest = event;
      hasSnapshot = true;
      controller.add(value);
    } catch (error, stack) {
      if (!disposed && (!hasSnapshot || error is MeCacheException)) {
        controller.addError(error, stack);
      }
    }
  }

  Future<void> refresh() async {
    try {
      final event = await service.head();
      if (disposed) return;
      if (event != null) {
        await accept(event);
      } else if (latest == null) {
        await service.cache(null);
        if (disposed || latest != null) return;
        hasSnapshot = true;
        controller.add(const MeGroups());
      }
    } catch (error, stack) {
      if (!disposed && (!hasSnapshot || error is MeCacheException)) {
        controller.addError(error, stack);
      }
    }
  }

  Future<void> start() async {
    try {
      final cached = service.cachedSnapshot();
      if (cached != null) {
        latest = cached.event;
        hasSnapshot = true;
        controller.add(cached.groups);
      }
    } catch (_) {
      // A corrupt cache must not prevent a fresh relay read.
    }
    try {
      service.check();
      unsubscribe = await service.session.subscribe(
        service.filter,
        accept,
        onClosed: (_) {
          if (!disposed && !hasSnapshot) {
            controller.addError(
              StateError('Couldn’t sync Solo. Reconnect and try again.'),
            );
          }
        },
      );
      if (disposed) {
        unsubscribe?.call();
        return;
      }
      await refresh();
    } catch (error, stack) {
      if (!disposed && !hasSnapshot) controller.addError(error, stack);
    }
  }

  ref.listen(appLifecycleProvider, (_, next) {
    if (next == AppLifecycleState.resumed) unawaited(refresh());
  });
  ref.onDispose(() {
    disposed = true;
    unsubscribe?.call();
    unawaited(controller.close());
  });
  unawaited(start());
  return controller.stream;
}, retry: (_, _) => null);

/// NIP-01 replacement ordering: smallest ID wins an equal timestamp.
bool meEventIsNewer(NostrEvent a, NostrEvent b) =>
    a.createdAt > b.createdAt ||
    (a.createdAt == b.createdAt && a.id.compareTo(b.id) < 0);

final meSyncProvider = Provider<MeSync>((ref) {
  final config = ref.watch(relayConfigProvider);
  final status = ref.watch(relaySessionProvider).status;
  final service = MeSync(
    ref.read(relaySessionProvider.notifier),
    config,
    connected: status == SessionStatus.connected,
    prefs: ref.read(savedPrefsProvider),
  );
  ref.onDispose(() => service.disposed = true);
  return service;
});

/// Narrow private-recipe transport; never writes Messages organization.
class MeSync {
  final RelaySessionNotifier session;
  final RelayConfig config;
  late final String community = Uri.parse(config.baseUrl).origin;
  late final String privateHex = nostr.Nip19.decode(payload: config.nsec!).data;
  late final String pubkey = nostr.Keys(privateHex).public;
  late final key = getConversationKey(privateHex, pubkey);
  final bool connected;
  final SharedPreferences? prefs;
  String get pendingKey => 'me-pending:$community:$pubkey';
  bool disposed = false;
  bool _saving = false;

  MeSync(this.session, this.config, {this.connected = true, this.prefs});

  /// Encrypted last verified organization, isolated by account and community.
  String get cacheKey => 'me-cache:$community:$pubkey';

  /// Read while offline. A signed event preserves replacement ordering.
  ({NostrEvent? event, MeGroups groups})? cachedSnapshot() {
    if (disposed || config.nsec == null) return null;
    final saved = prefs?.getString(cacheKey);
    if (saved == null) return null;
    final value = jsonDecode(nip44Decrypt(key, saved)) as Map<String, dynamic>;
    if (value['community'] != community) {
      throw const FormatException('Invalid cached organization');
    }
    final json = value['event'] as Map<String, dynamic>?;
    final event = json == null ? null : NostrEvent.fromJson(json);
    return (
      event: event,
      groups: event == null ? const MeGroups() : decode(event),
    );
  }

  Future<void> _cacheQueue = Future.value();
  bool _cacheDirty = false;

  /// Serialize writes and report failures so callers can expose a retry.
  Future<void> cache(NostrEvent? event) {
    final operation = _cacheQueue.then((_) => _writeCache(event));
    _cacheQueue = operation.catchError((Object _) {});
    return operation;
  }

  Future<void> _writeCache(NostrEvent? event) async {
    if (disposed || config.nsec == null || prefs == null) return;
    if (event != null) decode(event);
    try {
      final previous = cachedSnapshot()?.event;
      if (!_cacheDirty &&
          previous != null &&
          (event == null || !meEventIsNewer(event, previous))) {
        return;
      }
    } catch (_) {
      // Replace corrupt local storage with verified relay data.
    }
    try {
      if (!await prefs!.setString(
        cacheKey,
        nip44Encrypt(
          key,
          jsonEncode({'community': community, 'event': event?.toJson()}),
        ),
      )) {
        throw const MeCacheException();
      }
      _cacheDirty = false;
    } catch (_) {
      // SharedPreferences updates its memory cache even if the disk write fails.
      // Reload so Retry cannot mistake that optimistic value for durable data.
      _cacheDirty = true;
      try {
        await prefs!.reload();
      } finally {
        throw const MeCacheException();
      }
    }
  }

  NostrFilter get filter => NostrFilter(
    kinds: const [30078],
    authors: [pubkey],
    tags: {
      '#d': [MeGroups.coordinate(community)],
    },
    limit: 1,
  );

  void check() {
    if (!connected || config.nsec == null) {
      throw StateError('Connect to your community to load Solo.');
    }
    if (disposed) throw StateError('Your community changed. Open Solo again.');
  }

  MeGroups decode(NostrEvent event) {
    if (event.kind != 30078 ||
        event.pubkey != pubkey ||
        event.tags.where((t) => t.isNotEmpty && t[0] == 'd').length != 1 ||
        !event.tags.any(
          (t) =>
              t.length == 2 &&
              t[0] == 'd' &&
              t[1] == MeGroups.coordinate(community),
        ) ||
        !event.tags.any(
          (t) => t.length == 2 && t[0] == 't' && t[1] == MeGroups.tag,
        )) {
      throw const FormatException('Invalid Solo record');
    }
    nostr.Event.fromJson(jsonEncode(event.toJson()));
    return MeGroups.decode(nip44Decrypt(key, event.content), community);
  }

  Future<NostrEvent?> head() async {
    check();
    final events = await session.fetchHistory(filter);
    check();
    if (events.isEmpty) return null;
    final event = events.reduce((a, b) => meEventIsNewer(a, b) ? a : b);
    decode(event);
    return event;
  }

  /// Discard only the pending snapshot the user explicitly reviewed.
  Future<void> discardPending(String expected) async {
    check();
    if (_saving || prefs?.getString(pendingKey) != expected) {
      throw StateError('The pending save changed. Review it again.');
    }
    if (!await prefs!.remove(pendingKey)) {
      throw StateError('Couldn’t discard the pending save. Try again.');
    }
  }

  /// Refresh before applying an intent, then confirm the selected signed event.
  Future<void> update(MeGroups Function(MeGroups) change) async {
    check();
    if (_saving) throw StateError('A Solo change is still being saved.');
    _saving = true;
    try {
      MeGroups? recovered;
      String? recoveredHead;
      final pending = prefs?.getString(pendingKey);
      if (pending != null) {
        final stored = jsonDecode(pending) as Map<String, dynamic>;
        final event = NostrEvent.fromJson(
          (stored['event'] as Map<String, dynamic>?) ?? stored,
        );
        final local = decode(event);
        final remote = await head();
        if (remote != null &&
            remote.id != event.id &&
            meEventIsNewer(remote, event)) {
          if (!stored.containsKey('event')) {
            // Older clients did not persist a base, so a safe rebase is unknown.
            throw MeRecoveryConflict();
          }
          recoveredHead = remote.id;
          final baseEvent = stored['base'] as Map<String, dynamic>?;
          recovered = rebaseMeGroups(
            baseEvent == null
                ? const MeGroups()
                : decode(NostrEvent.fromJson(baseEvent)),
            local,
            decode(remote),
          );
        } else {
          if (remote?.id != event.id) {
            check();
            await session.publish(event);
            if ((await head())?.id != event.id) {
              throw StateError(
                'A previous Solo save is still unconfirmed. Try again.',
              );
            }
          }
          await prefs?.remove(pendingKey);
          throw StateError(
            'Your previous Solo save is confirmed. Review your projects before making another change.',
          );
        }
      }
      final before = await head();
      if (recovered != null && before?.id != recoveredHead) {
        throw StateError(
          'Solo changed again. Your pending change is saved; try again.',
        );
      }
      final value =
          recovered ??
          change(before == null ? const MeGroups() : decode(before));
      final String encoded;
      try {
        encoded = value.encode(community);
      } on FormatException {
        if (recovered != null) throw MeRecoveryConflict();
        rethrow;
      }
      final content = nip44Encrypt(key, encoded);
      final current = await head();
      if (before?.id != current?.id) {
        throw StateError(
          'Solo changed on another device. Refresh and try again.',
        );
      }
      if (current != null &&
          current.createdAt >= DateTime.now().millisecondsSinceEpoch ~/ 1000) {
        throw StateError('Please wait a second before saving again.');
      }
      check();
      final signed = NostrEvent.fromJson(
        nostr.Event.from(
          kind: 30078,
          content: content,
          secretKey: privateHex,
          verify: false,
          tags: [
            ['d', MeGroups.coordinate(community)],
            ['t', MeGroups.tag],
          ],
        ).toMap(),
      );
      if (prefs != null &&
          !await prefs!.setString(
            pendingKey,
            jsonEncode({'event': signed.toJson(), 'base': before?.toJson()}),
          )) {
        throw StateError('Couldn’t save this change on your phone. Try again.');
      }
      check();
      await session.publish(signed);
      check();
      if ((await head())?.id != signed.id) {
        throw StateError(
          'Another Solo change is selected. Refresh to see the latest projects.',
        );
      }
      await prefs?.remove(pendingKey);
      if (recovered != null) {
        throw StateError(
          'Your previous Solo save was recovered alongside the desktop changes. Review your projects before making another change.',
        );
      }
    } finally {
      _saving = false;
    }
  }
}

/// A verified Solo snapshot could not be retained for offline use.
class MeCacheException implements Exception {
  const MeCacheException();
  @override
  String toString() => 'Could not save Solo for offline use. Please retry.';
}
