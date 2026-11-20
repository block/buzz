import 'dart:isolate';

import 'package:flutter/scheduler.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/channels/saved_message_store.dart';
import '../../shared/relay/relay.dart';
import '../../shared/relay/relay_author_provider.dart';
import 'channel_event_order.dart';
import 'channel_window.dart';

typedef SavedChannelPreview = ({
  List<NostrEvent> events,
  Map<String, ChannelWindowThreadSummary> summaries,
});

/// A disk page goes straight to the view. The live notifier can replace the
/// preview without ever folding its old rows into current relay history.
final savedChannelPreviewProvider = FutureProvider.autoDispose
    .family<SavedChannelPreview?, String>((ref, channelId) async {
      final scope = ref.watch(savedMessageScopeProvider);
      if (scope == null) return null;
      final cache = ref.watch(savedMessageStoreProvider);
      var disposed = false;
      ref.onDispose(() => disposed = true);
      if (ref.read(relaySessionProvider).status == SessionStatus.connected) {
        await Future<void>.delayed(const Duration(milliseconds: 250));
      }
      // A covered route can pause its listeners while the read finishes.
      // Retry retired tickets until this provider is disposed.
      while (!disposed) {
        final ticket = cache.capture(scope, (channelId: channelId, rootId: ''));
        final events = await cache.read(ticket);
        if (disposed) return null;
        if (!cache.isCurrent(ticket)) continue;
        if (events == null) return null;
        final preview = await _preparePreview(events);
        if (disposed) return null;
        if (cache.isCurrent(ticket)) return preview;
      }
      return null;
    });

Future<SavedChannelPreview> _preparePreview(List<NostrEvent> events) =>
    Isolate.run(() {
      final rows = <NostrEvent>[];
      final summaries = <String, ChannelWindowThreadSummary>{};
      for (final event in events) {
        if (event.kind == EventKind.channelThreadSummary) {
          // A malformed summary must not make its verified message unreadable.
          try {
            summaries[event.getTagValue('e')!] =
                parseChannelWindowThreadSummary(event);
          } catch (_) {
            continue;
          }
        } else {
          rows.add(event);
        }
      }
      return (
        events: rows..sort(compareChannelTimelineEventsChronologically),
        summaries: summaries,
      );
    });

final _savedMessageRelayAuthorProvider = FutureProvider<String?>((ref) async {
  final connected = ref.watch(
    relaySessionProvider.select(
      (state) => state.status == SessionStatus.connected,
    ),
  );
  if (!connected) return null;
  final origin = ref.watch(
    relayConfigProvider.select((config) => config.baseUrl),
  );
  try {
    return await ref.watch(relayAuthorProvider(origin).future);
  } catch (_) {
    return null;
  }
});

/// Persists only signed events that came from the completed relay read.
Future<void> saveChannelPage(
  Ref ref,
  SavedMessageStore store,
  SavedMessageTicket ticket,
  List<NostrEvent> events, {
  required bool Function() isCurrent,
}) async {
  try {
    final root = await ref.read(savedMessageRootProvider.future);
    if (root == null || !isCurrent() || !store.isCurrent(ticket)) return;
    // Let the relay timeline paint before copying the page to the worker.
    await SchedulerBinding.instance.endOfFrame;
    if (!isCurrent() || !store.isCurrent(ticket)) return;
    final author = events.isEmpty
        ? null
        : await ref.read(_savedMessageRelayAuthorProvider.future);
    if (!isCurrent() || !store.isCurrent(ticket)) return;
    await store.save(ticket, events, relayAuthor: author);
  } catch (_) {
    // Disk readiness and relay discovery are outside the live timeline path.
  }
}
