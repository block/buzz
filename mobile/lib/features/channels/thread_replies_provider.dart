import 'dart:async';

import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:hooks_riverpod/misc.dart' show KeepAliveLink;

import '../../shared/channels/saved_message_store.dart';
import '../../shared/relay/relay.dart';
import 'channel_event_order.dart';
import 'channel_messages_provider.dart';
import 'pending_local_messages_provider.dart';
import 'saved_channel_page.dart';

const _threadReplyPageSize = 200;

class ThreadRepliesArgs {
  final String channelId;
  final String rootId;

  const ThreadRepliesArgs({required this.channelId, required this.rootId});

  SavedMessagePage get savedPage => (channelId: channelId, rootId: rootId);

  @override
  bool operator ==(Object other) =>
      identical(this, other) ||
      other is ThreadRepliesArgs &&
          channelId == other.channelId &&
          rootId == other.rootId;

  @override
  int get hashCode => Object.hash(channelId, rootId);
}

class _ThreadCursor {
  final int createdAt;
  final String eventId;

  const _ThreadCursor({required this.createdAt, required this.eventId});

  /// Whether this sorts after [other] in the relay's thread order: time, then
  /// event id (lowercase hex, so string order is byte order).
  bool isAfter(_ThreadCursor other) =>
      createdAt > other.createdAt ||
      (createdAt == other.createdAt && eventId.compareTo(other.eventId) > 0);
}

// Keep the save budget across live and reconnect rebuilds. It resets when
// the thread closes or the cache scope changes.
final _claimThreadSaveProvider = Provider.autoDispose
    .family<bool Function(), ThreadRepliesArgs>((ref, args) {
      ref.watch(savedMessageScopeProvider);
      var claimed = false;
      return () {
        if (claimed) return false;
        claimed = true;
        return true;
      };
    });

final threadRepliesProvider = FutureProvider.autoDispose
    .family<List<NostrEvent>, ThreadRepliesArgs>((ref, args) async {
      // A reply missed while the socket is stale cannot invalidate this
      // one-shot query. Refresh mounted threads when the session recovers;
      // auto-dispose also makes reopening a thread start from relay truth.
      ref.listen(relaySessionProvider, (previous, next) {
        if (previous?.status != SessionStatus.connected &&
            next.status == SessionStatus.connected) {
          ref.invalidateSelf();
        }
      });
      final session = ref.read(relaySessionProvider.notifier);
      final scope = ref.watch(savedMessageScopeProvider);
      final cache = ref.read(savedMessageStoreProvider);
      final ticket = scope == null
          ? null
          : cache.capture(scope, args.savedPage);
      final claimSave = ref.watch(_claimThreadSaveProvider(args));
      final channelProvider = channelMessagesProvider(args.channelId);
      final channelMessages = ref.exists(channelProvider)
          ? ref.read(channelProvider.notifier)
          : null;
      void denyAccess(Object error, StackTrace trace) {
        if (ref.exists(channelProvider)) {
          ref.read(channelProvider.notifier).denyAccessFromThread(error, trace);
        } else if (scope != null) {
          unawaited(cache.removeChannel(scope, args.channelId));
          invalidateSavedMessagePreviews(ref, args.channelId);
        }
      }

      final cachedReplyIds = channelMessages?.cachedThreadReplyIds(args.rootId);
      final unconfirmedIds = channelMessages?.unconfirmedThreadReplyIds(
        args.rootId,
      );
      final queryVersion = channelMessages?.beginThreadQuery(args.rootId);
      // Riverpod can invalidate this build while its element remains mounted.
      var current = true;
      ref.onDispose(() {
        current = false;
        if (queryVersion != null) {
          channelMessages?.failThreadQuery(args.rootId, queryVersion);
        }
      });
      try {
        // Edits, reactions and deletions of the root and its replies ride
        // along, so a reply edited before the thread opens shows that edit.
        // Only the replies themselves feed the reply cache below.
        final (:replies, :aux) = await fetchCompleteThread(
          session,
          args,
          isCurrent: () => current,
        );
        if (!current) return const [];
        // Explicit markers also settle unacknowledged local sends, whose
        // absence cannot prove deletion even in an insertion-complete scan.
        final missingIds =
            unconfirmedIds?.difference(
              replies.map((event) => event.id).toSet(),
            ) ??
            <String>{};
        final deletions = <NostrEvent>[];
        // One bounded request per scan keeps opening a thread responsive even
        // when many sends are awaiting ACKs. Excess IDs remain provisional.
        final targets =
            channelMessages?.nextThreadDeletionProofTargets(missingIds) ??
            const <String>[];
        // Per-target limits keep repeated markers from crowding another ID out.
        if (targets.isNotEmpty) {
          deletions.addAll(
            await session.queryRelay([
              for (final target in targets)
                NostrFilter(
                  kinds: const [EventKind.deletion, EventKind.nip29DeleteEvent],
                  tags: {
                    '#h': [args.channelId],
                    '#e': [target],
                  },
                  limit: 1,
                ),
            ]),
          );
        }
        if (!current) return const [];
        if (ref.exists(channelProvider)) {
          final channel = ref.read(channelProvider.notifier);
          final deletedTargets = {
            for (final event in deletions)
              for (final tag in event.tags)
                if (tag.length > 1 && tag[0] == 'e') tag[1],
          };
          final applied = channel.cacheCompleteThreadQuery(
            args.rootId,
            cachedReplyIds ?? {},
            replies,
            provisionalReplyIds: missingIds.difference(deletedTargets),
            queryVersion: queryVersion,
          );
          if (deletions.isNotEmpty) {
            channel.cacheThreadDeletions(
              deletions,
              scopedTargetIds: targets.toSet(),
              reconciledTargetIds: applied ? missingIds : const {},
            );
          }
        }
        final events = [...replies, ...aux];
        if (current &&
            ticket != null &&
            channelMessages?.isAccessDenied != true) {
          final rootPending = ref
              .read(pendingLocalMessagesProvider(args.channelId))
              .containsKey(args.rootId);
          final knownRoot = ref.exists(channelProvider) && !rootPending
              ? ref
                    .read(channelProvider)
                    .value
                    ?.where((event) => event.id == args.rootId)
                    .firstOrNull
              : null;
          unawaited(
            saveMessagePage(
              ref,
              cache,
              ticket,
              events,
              isCurrent: () =>
                  current && channelMessages?.isAccessDenied != true,
              refreshEvents: () async {
                if (!claimSave()) return null;
                try {
                  return await _readSavedThreadEvents(
                    session,
                    args,
                    events,
                    cache,
                    ticket,
                    knownRoot,
                    isCurrent: () => current && cache.isCurrent(ticket),
                  );
                } catch (error, trace) {
                  if (current &&
                      cache.isCurrent(ticket) &&
                      isRelayChannelAccessDenied(error)) {
                    denyAccess(error, trace);
                  }
                  rethrow;
                }
              },
            ),
          );
        }
        return events;
      } catch (error, stackTrace) {
        if (current && isRelayChannelAccessDenied(error)) {
          denyAccess(error, stackTrace);
        }
        if (current && queryVersion != null) {
          channelMessages?.failThreadQuery(args.rootId, queryVersion);
        }
        rethrow;
      }
    });

Future<List<NostrEvent>> _readSavedThreadEvents(
  RelaySessionNotifier session,
  ThreadRepliesArgs args,
  List<NostrEvent> events,
  SavedMessageStore store,
  SavedMessageTicket ticket,
  NostrEvent? knownRoot, {
  required bool Function() isCurrent,
}) async {
  if (events.length > SavedMessageStore.maxPageEvents) return const [];
  final targets = {
    args.rootId,
    for (final event in events)
      if (EventKind.channelTimelineContentKinds.contains(event.kind)) event.id,
  };
  // Bound the writer query to the root and one reply page. A larger thread
  // still has complete relay history, but cannot replace its saved page
  // without also verifying its edits.
  if (targets.length > _threadReplyPageSize + 1) return const [];
  final prior = await store.inspectThread(ticket);
  if (!isCurrent()) return const [];
  String? editTarget(NostrEvent event) => event.tags.reversed
      .where((tag) => tag.length > 1 && tag[0] == 'e')
      .firstOrNull?[1];
  final (:edited, :deleted) = store.threadMutations(ticket, {
    args.rootId,
    ...events.map((event) => event.id),
  });
  final pruned = prior?.pruned ?? const <String, bool>{};
  deleted.addAll({
    for (final entry in pruned.entries)
      if (!entry.value) entry.key,
    for (final event in prior?.events ?? const <NostrEvent>[])
      if (event.kind == EventKind.deletion ||
          event.kind == EventKind.nip29DeleteEvent)
        for (final tag in event.tags)
          if (tag.length > 1 && tag[0] == 'e') tag[1],
  });
  final prunedEdits = targets.where((id) => pruned[id] == true).toSet();
  final idsToVerify = {if (knownRoot == null) args.rootId, ...prunedEdits}
    ..removeAll(deleted);
  final editTargets = targets.intersection({
    ...edited,
    ...prunedEdits,
    for (final event in prior?.events ?? const <NostrEvent>[])
      if (event.kind == EventKind.streamMessageEdit) ?editTarget(event),
  })..removeAll(deleted);
  // Verify only mutations observed live or in a previous saved page.
  const editLimit = 1000;
  const strong = {'consistency': 'strong'};
  final filters = [
    if (idsToVerify.isNotEmpty)
      NostrFilter(
        kinds: EventKind.channelTimelineContentKinds,
        ids: idsToVerify.toList(),
        tags: {
          '#h': [args.channelId],
        },
        limit: idsToVerify.length,
        extensions: strong,
      ),
    if (editTargets.isNotEmpty)
      NostrFilter(
        kinds: const [EventKind.streamMessageEdit],
        tags: {
          '#h': [args.channelId],
          '#e': editTargets.toList(),
        },
        limit: editLimit,
        extensions: strong,
      ),
  ];
  final current = filters.isEmpty
      ? const <NostrEvent>[]
      : await session.queryRelay(filters);
  final writerEdits = current
      .where((event) => event.kind == EventKind.streamMessageEdit)
      .toList();
  // At the limit the response may omit an edit. Clear the whole saved page.
  if (writerEdits.length >= editLimit) return const [];
  final verifiedRows = current
      .where(
        (event) =>
            idsToVerify.contains(event.id) &&
            EventKind.channelTimelineContentKinds.contains(event.kind),
      )
      .toList();
  return [
    if (knownRoot != null &&
        !deleted.contains(knownRoot.id) &&
        !idsToVerify.contains(knownRoot.id))
      knownRoot,
    ...verifiedRows,
    ...writerEdits.where((event) => !deleted.contains(event.id)),
    for (final event in events)
      if (!deleted.contains(event.id) &&
          !idsToVerify.contains(event.id) &&
          !(event.kind == EventKind.streamMessageEdit &&
              editTargets.contains(editTarget(event))))
        event,
  ];
}

/// Exhaustively scans a thread using insertion-complete cursor pages.
/// [isCurrent] lets a background refresh stop between pages after disposal.
Future<List<NostrEvent>> fetchCompleteThreadReplies(
  RelaySessionNotifier session,
  ThreadRepliesArgs args, {
  bool Function()? isCurrent,
}) async => (await fetchCompleteThread(
  session,
  args,
  isCurrent: isCurrent,
  includeAux: false,
)).replies;

/// [fetchCompleteThreadReplies], plus the edits, reactions and deletions the
/// relay attaches to the root and each reply page when [includeAux] is set.
/// Pages are counted and resumed by their replies alone.
Future<({List<NostrEvent> replies, List<NostrEvent> aux})> fetchCompleteThread(
  RelaySessionNotifier session,
  ThreadRepliesArgs args, {
  bool Function()? isCurrent,
  bool includeAux = true,
}) async {
  final replies = <NostrEvent>[];
  final aux = <NostrEvent>[];
  final replyIds = <String>{};
  final auxIds = <String>{};
  // -1 precedes unsigned Nostr timestamps. A non-null cursor selects the
  // insertion-complete route and writer-verified EOF instead of a stale head.
  _ThreadCursor? cursor = const _ThreadCursor(
    createdAt: -1,
    eventId: '0000000000000000000000000000000000000000000000000000000000000000',
  );
  for (var page = 0; page < 500; page++) {
    if (isCurrent != null && !isCurrent()) {
      throw StateError('Thread scan superseded');
    }
    final events = await session.queryRelay([
      _threadRepliesFilter(args, cursor, includeAux: includeAux),
    ]);
    final rows = includeAux ? _leadingRows(events, args.rootId) : events;
    for (final row in rows) {
      if (!includeAux || replyIds.add(row.id)) replies.add(row);
    }
    for (final event in events.skip(rows.length)) {
      if (auxIds.add(event.id)) aux.add(event);
    }
    if (rows.length < _threadReplyPageSize) return (replies: replies, aux: aux);
    final last = rows.last;
    final next = _ThreadCursor(createdAt: last.createdAt, eventId: last.id);
    // A full page of real rows always moves past the cursor. One that does
    // not was a short final page whose aux looked like rows.
    if (includeAux && !next.isAfter(cursor!)) {
      return (replies: replies, aux: aux);
    }
    cursor = next;
  }
  throw Exception('Thread ${args.rootId} exceeded the page safety limit.');
}

/// The rows of one `include_aux` thread page. The relay sends a page's rows,
/// each a reply-marked event and at most 200, before the aux closure. The
/// closure repeats the root's aux on every page, and a reply-marked edit or
/// NIP-29 deletion there is also a row of some page, so the rows are the
/// leading run of events reply-marked into this thread. Reactions and NIP-09
/// deletions are never thread rows.
List<NostrEvent> _leadingRows(List<NostrEvent> events, String rootId) {
  var count = 0;
  while (count < events.length &&
      count < _threadReplyPageSize &&
      events[count].threadReference.parentId != null &&
      events[count].threadReference.rootId == rootId &&
      events[count].kind != EventKind.reaction &&
      events[count].kind != EventKind.deletion) {
    count++;
  }
  return events.sublist(0, count);
}

NostrFilter _threadRepliesFilter(
  ThreadRepliesArgs args,
  _ThreadCursor? cursor, {
  required bool includeAux,
}) {
  return NostrFilter(
    kinds: EventKind.channelTimelineContentKinds,
    tags: {
      '#e': [args.rootId],
      '#h': [args.channelId],
    },
    limit: _threadReplyPageSize,
    extensions: {
      // The relay binds this as signed i32. Include every representable depth.
      'depth_limit': 0x7fffffff,
      if (includeAux) 'include_aux': true,
      if (cursor != null) 'thread_cursor': cursor.createdAt,
      if (cursor != null) 'thread_cursor_id': cursor.eventId,
    },
  );
}

class ThreadLocalRepliesNotifier extends Notifier<List<NostrEvent>> {
  final ThreadRepliesArgs args;

  ThreadLocalRepliesNotifier(this.args);

  @override
  List<NostrEvent> build() {
    // Keep optimistic replies across route disposal, but release empty overlays.
    KeepAliveLink? retention;
    listenSelf((previous, next) {
      if (next.isNotEmpty) {
        retention ??= ref.keepAlive();
      } else {
        retention?.close();
        retention = null;
      }
    });
    return const [];
  }

  void add(NostrEvent event) {
    state = _mergeReplies(state, [event]);
  }

  void remove(String eventId) {
    state = state.where((event) => event.id != eventId).toList();
  }

  void confirm(Set<String> eventIds) {
    if (!state.any((event) => eventIds.contains(event.id))) return;
    state = state.where((event) => !eventIds.contains(event.id)).toList();
  }
}

final threadLocalRepliesProvider = NotifierProvider.autoDispose
    .family<ThreadLocalRepliesNotifier, List<NostrEvent>, ThreadRepliesArgs>(
      ThreadLocalRepliesNotifier.new,
    );

/// Relay-backed replies merged with signed local replies that are still
/// waiting for acknowledgement.
///
/// The relay query is route-scoped, while the optimistic local overlay stays
/// alive until confirmation so it can survive closing and reopening a thread.
final threadRepliesWithLocalProvider = Provider.autoDispose
    .family<AsyncValue<List<NostrEvent>>, ThreadRepliesArgs>((ref, args) {
      final relayReplies = ref.watch(threadRepliesProvider(args));
      final localReplies = ref.watch(threadLocalRepliesProvider(args));
      final authoritative = relayReplies.value;
      if (authoritative != null && localReplies.isNotEmpty) {
        final authoritativeIds = authoritative.map((event) => event.id).toSet();
        if (localReplies.any((event) => authoritativeIds.contains(event.id))) {
          final localRepliesNotifier = ref.read(
            threadLocalRepliesProvider(args).notifier,
          );
          final pendingMessagesNotifier = ref.read(
            pendingLocalMessagesProvider(args.channelId).notifier,
          );
          final channelMessages = ref.read(
            channelMessagesProvider(args.channelId).notifier,
          );
          final confirmedReplies = authoritative
              .where(
                (event) => localReplies.any((local) => local.id == event.id),
              )
              .toList();
          Future.microtask(() {
            channelMessages.cacheConfirmedThreadReplies(confirmedReplies);
            localRepliesNotifier.confirm(authoritativeIds);
            pendingMessagesNotifier.confirm(authoritativeIds);
          });
        }
      }
      if (localReplies.isEmpty) return relayReplies;
      // This provider supplies display events; the original query remains
      // the source of loading/error status. Preserve its retained value when
      // merging optimistic replies during a failed refresh or retry.
      return AsyncData(_mergeReplies(authoritative ?? const [], localReplies));
    });

/// Union two event lists by id, newest-wins, in timeline order.
///
/// The thread view needs this to fold the channel's live socket events into its
/// own one-shot query result: the query returns the reactions, edits, and
/// deletions that existed when it ran, so ones that land while a thread is
/// open never reach it on their own.
List<NostrEvent> mergeThreadEvents(
  Iterable<NostrEvent> first,
  Iterable<NostrEvent> second,
) => _mergeReplies(first, second);

List<NostrEvent> _mergeReplies(
  Iterable<NostrEvent> first,
  Iterable<NostrEvent> second,
) {
  final byId = <String, NostrEvent>{};
  for (final event in [...first, ...second]) {
    byId[event.id] = event;
  }
  return byId.values.toList()..sort(compareThreadRepliesChronologically);
}
