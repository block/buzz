import '../../shared/agents/is_quiet_agent_message.dart';
import '../../shared/mentions/mention_tags.dart';
import '../../shared/relay/relay.dart';
import '../../shared/relay/conversation_scope.dart';
import 'activity_messages.dart';
import 'activity_queries.dart';
import 'feed_item.dart';

typedef ActivityQuery = Future<List<NostrEvent>> Function(List<NostrFilter>);

/// Session-scoped retained evidence; a reconnect reads only its delta.
class ActivityHistoryCache {
  Map<String, NostrEvent> events = {};
  Set<String> channels = {};
  Set<String> interests = {};
  int? since;
}

/// Conversation evidence for Activity, including replies without a p-tag.
/// Cold reads aggregate channel scopes under the relay cap. Retained evidence
/// and delta cursors keep reconnects from rebuilding channel history.
Future<List<FeedItem>> loadActivityHistory({
  required ActivityQuery query,
  required List<ConversationScope> channels,
  required String viewer,
  required Set<String> followedRoots,
  List<NostrEvent> seedEvents = const [],
  ActivityHistoryCache? cache,
  bool Function()? isCurrent,
}) async {
  final allowed = {
    for (final channel in channels)
      if (channel.isMember &&
          !channel.isArchived &&
          channel.channelType != 'system')
        channel.id: channel,
  };
  if (allowed.isEmpty) return [];
  final startedAt = DateTime.now().millisecondsSinceEpoch ~/ 1000;
  final retained = cache ?? ActivityHistoryCache();
  final existingIds = retained.events.keys.toSet();
  final existingChannels = allowed.keys
      .where(retained.channels.contains)
      .toList();
  const kinds = activityMessageKinds;
  final filters = [
    // Aggregate cold windows: 300 memberships require three reads, not 300.
    for (final dm in [true, false])
      ..._coldFilters(
        allowed.values
            .where((c) => c.isDm == dm && !retained.channels.contains(c.id))
            .map((c) => c.id)
            .toList(),
      ),
    // Reconnect/live catch-up batches the known roster under the relay cap.
    for (
      var start = 0;
      start < existingChannels.length;
      start += kMaxExplicitChannelValues
    )
      NostrFilter(
        kinds: const [...activityMessageKinds, 40003, 5, 9005, 7],
        tags: {
          '#h': existingChannels
              .skip(start)
              .take(kMaxExplicitChannelValues)
              .toList(),
        },
        since: retained.since,
        limit: 500,
      ),
    // Channel-less NIP-09 deletions also need a reconnect delta.
    for (var start = 0; start < existingIds.length; start += 500)
      NostrFilter(
        kinds: const [5, 9005],
        tags: {'#e': existingIds.skip(start).take(500).toList()},
        since: retained.since,
        limit: 500,
      ),
    NostrFilter(
      kinds: [7],
      tags: {
        '#p': [viewer],
      },
      since: retained.since,
      limit: 50,
    ),
  ];
  final events = <String, NostrEvent>{
    for (final event in [...retained.events.values, ...seedEvents])
      if (allowed.containsKey(event.channelId) ||
          event.kind == 7 ||
          event.channelId == null)
        event.id: event,
  };

  for (var start = 0; start < filters.length;) {
    final batch = <NostrFilter>[];
    var explicitChannels = 0;
    while (start < filters.length && batch.length < 4) {
      final next = filters[start];
      final size = next.tags['#h']?.length ?? 0;
      if (batch.isNotEmpty &&
          explicitChannels + size > kMaxExplicitChannelValues) {
        break;
      }
      batch.add(next);
      explicitChannels += size;
      start++;
    }
    final received = await query(batch);
    // Page a full delta before advancing its cursor; otherwise an offline
    // burst can permanently skip the oldest events in that burst.
    for (final filter in batch.where((f) => f.since != null)) {
      var page = received.where((e) => _matches(filter, e)).toList()
        ..sort(_newest);
      page = page.take(filter.limit).toList();
      for (var count = 0; page.length >= filter.limit; count++) {
        if (count == 20) {
          throw StateError(
            'Activity catch-up exceeds its budget. Refresh Activity.',
          );
        }
        final last = page.last;
        page = await query([
          NostrFilter(
            kinds: filter.kinds,
            tags: filter.tags,
            since: filter.since,
            until: last.createdAt,
            limit: filter.limit,
            extensions: {'before_id': last.id},
          ),
        ]);
        if (page.any(
          (e) =>
              e.createdAt > last.createdAt ||
              (e.createdAt == last.createdAt && e.id.compareTo(last.id) <= 0),
        )) {
          throw StateError('Activity catch-up did not advance.');
        }
        page.sort(_newest);
        received.addAll(page);
      }
    }
    for (final event in received) {
      if (allowed.containsKey(event.channelId) ||
          event.kind == 7 ||
          (event.channelId == null && event.kind == 5)) {
        events[event.id] = event;
      }
    }
  }
  _trimEvidence(events);
  // Resolve exact parents/roots and reaction targets, including messages older
  // than the channel window. A tag alone does not establish ownership.
  final targets = <String>{};
  for (final event in events.values) {
    if (event.kind == 7) {
      final target = _reactionTarget(event);
      if (target != null) targets.add(target);
    } else {
      final thread = event.threadReference;
      if (thread.rootId != null) targets.add(thread.rootId!);
      if (thread.parentId != null) targets.add(thread.parentId!);
    }
  }
  final missing = targets.difference(events.keys.toSet()).toList();
  for (var start = 0; start < missing.length; start += 100) {
    for (final event in await query([
      NostrFilter(
        kinds: kinds,
        ids: missing.skip(start).take(100).toList(),
        limit: 100,
      ),
    ])) {
      if (allowed.containsKey(event.channelId)) events[event.id] = event;
    }
  }
  // Participation may predate every channel window. Ask for the viewer's
  // replies to the exact candidate parents/roots instead of scanning a global
  // latest-authored window, where busy conversations can hide that evidence.
  final interests = <String, Set<String>>{};
  for (final event in events.values.toList()) {
    if (event.kind == 7 || !allowed.containsKey(event.channelId)) continue;
    final thread = event.threadReference;
    if (thread.parentId == null) continue;
    interests.putIfAbsent(event.channelId!, () => {}).addAll([
      thread.parentId!,
      if (thread.rootId != null) thread.rootId!,
    ]);
  }
  final newTargets = {
    for (final entry in interests.entries)
      for (final target in entry.value)
        if (!retained.interests.contains('${entry.key}:$target')) target,
  }.toList();
  final interestFilters = [
    for (var start = 0; start < newTargets.length; start += 500)
      NostrFilter(
        kinds: kinds,
        authors: [viewer],
        tags: {'#e': newTargets.skip(start).take(500).toList()},
        limit: 500,
      ),
  ];
  for (var start = 0; start < interestFilters.length; start += 4) {
    for (final event in await queryActivityEvidence(
      query,
      interestFilters.skip(start).take(4).toList(),
    )) {
      if (allowed.containsKey(event.channelId) &&
          event.pubkey.toLowerCase() == viewer.toLowerCase()) {
        events[event.id] = event;
      }
    }
  }
  _trimEvidence(events);
  final overlays = [
    for (final event in events.values)
      if (const [40003, 5, 9005].contains(event.kind)) event,
    ...await _updates(
      query,
      events.keys.where((id) => !existingIds.contains(id)).toList(),
    ),
  ];
  final editDeletes = await _updates(
    query,
    [
      for (final event in overlays)
        if (event.kind == 40003 && !existingIds.contains(event.id)) event.id,
    ],
    kinds: const [5, 9005],
  );
  final all = {
    ...events,
    for (final e in [...overlays, ...editDeletes]) e.id: e,
  };
  final validUpdates = [
    for (final update in [...overlays, ...editDeletes])
      if (_validUpdate(update, all)) update,
  ];
  final deleted = {
    for (final update in validUpdates)
      if (update.kind == 5 || update.kind == 9005)
        for (final tag in update.tags)
          if (tag.length > 1 && tag[0] == 'e') tag[1],
  };
  final messages = {
    for (final message in activityMessages(events.values, validUpdates))
      message.id: message,
  };
  final ownThreads = <String>{};
  final ownParents = <String>{};
  String key(String? channel, String id) => '$channel:$id';
  for (final event in events.values) {
    if (!messages.containsKey(event.id) ||
        event.pubkey.toLowerCase() != viewer.toLowerCase()) {
      continue;
    }
    ownParents.add(key(event.channelId, event.id));
    ownThreads.add(key(event.channelId, event.id));
    final thread = event.threadReference;
    if (thread.rootId != null) {
      ownThreads.add(key(event.channelId, thread.rootId!));
    }
    if (thread.parentId != null) {
      ownParents.add(key(event.channelId, thread.parentId!));
    }
  }
  final result = <FeedItem>[];
  FeedItem item(NostrEvent event, String category) {
    final message = messages[event.id]!;
    final channel = allowed[event.channelId]!;
    return FeedItem(
      id: event.id,
      kind: event.kind,
      pubkey: message.pubkey,
      content: message.content,
      createdAt: event.createdAt,
      channelId: event.channelId,
      channelName: channel.name,
      tags: event.tags,
      category: category,
      channelCatchUp:
          !channel.isDm &&
          event.threadReference.parentId == null &&
          category != 'mention' &&
          !event.tags.any(
            (t) => t.length > 1 && t[0] == 'broadcast' && t[1] == '1',
          ),
    );
  }

  for (final event in events.values) {
    final channel = allowed[event.channelId];
    final message = messages[event.id];
    if (channel == null ||
        message == null ||
        event.pubkey.toLowerCase() == viewer.toLowerCase() ||
        isQuietAgentMessage(event.tags) ||
        isQuietAgentMessage(message.tags)) {
      continue;
    }
    final thread = event.threadReference;
    final mentioned = !channel.isDm && tagsAddress(message.tags, viewer);
    final relevantThread =
        thread.parentId != null &&
        ((thread.rootId != null &&
                (followedRoots.contains(thread.rootId) ||
                    ownThreads.contains(
                      key(event.channelId, thread.rootId!),
                    ))) ||
            ownParents.contains(key(event.channelId, thread.parentId!)));
    if (channel.isDm ||
        mentioned ||
        relevantThread ||
        (channel.isSolo && thread.parentId == null)) {
      result.add(item(event, mentioned ? 'mention' : 'activity'));
    }
  }
  for (final reaction in events.values) {
    if (reaction.kind != 7 ||
        deleted.contains(reaction.id) ||
        reaction.pubkey.toLowerCase() == viewer.toLowerCase()) {
      continue;
    }
    final target = events[_reactionTarget(reaction)];
    if (target == null ||
        target.pubkey.toLowerCase() != viewer.toLowerCase() ||
        !messages.containsKey(target.id) ||
        !allowed.containsKey(target.channelId) ||
        (reaction.channelId != null &&
            reaction.channelId != target.channelId) ||
        reaction.content.trim().isEmpty) {
      continue;
    }
    final targetItem = item(target, 'activity');
    result.add(
      FeedItem(
        id: reaction.id,
        kind: reaction.kind,
        pubkey: reaction.pubkey,
        content: targetItem.content,
        createdAt: reaction.createdAt,
        channelId: target.channelId,
        channelName: targetItem.channelName,
        tags: reaction.tags,
        category: 'reaction',
        reactionTarget: targetItem,
        reactionEmoji: reaction.content.trim(),
      ),
    );
  }
  if (isCurrent?.call() ?? true) {
    final evidence = all.values.toList()
      ..sort((a, b) => b.createdAt.compareTo(a.createdAt));
    // Retain a bounded window. Eviction forces a fresh bounded read next time,
    // rather than pretending an incremental cursor still covers lost evidence.
    final overflow = evidence.length > 4096;
    retained.events = {
      for (final event in evidence.take(4096)) event.id: event,
    };
    retained.channels = overflow ? {} : allowed.keys.toSet();
    retained.interests = overflow
        ? {}
        : {
            ...retained.interests,
            for (final entry in interests.entries)
              for (final target in entry.value) '${entry.key}:$target',
          };
    if (retained.interests.length > 4096) retained.interests.clear();
    if (!overflow) retained.since = startedAt - 5;
  }
  return result;
}

String? _reactionTarget(NostrEvent event) {
  final tags = event.tags.where((t) => t.length > 1 && t[0] == 'e').toList();
  return tags.length == 1 && RegExp(r'^[0-9a-f]{64}$').hasMatch(tags.single[1])
      ? tags.single[1]
      : null;
}

bool _validUpdate(NostrEvent update, Map<String, NostrEvent> events) {
  final ids = update.tags
      .where((t) => t.length > 1 && t[0] == 'e')
      .map((t) => t[1]);
  final knownIds = ids.where(events.containsKey).toList();
  return knownIds.isNotEmpty &&
      knownIds.every((id) {
        final target = events[id];
        return target != null &&
            (update.channelId == null ||
                update.channelId == target.channelId) &&
            (update.kind == 9005 || update.pubkey == target.pubkey);
      });
}

Future<List<NostrEvent>> _updates(
  ActivityQuery query,
  List<String> ids, {
  List<int> kinds = const [40003, 5, 9005],
}) async {
  final result = <NostrEvent>[];
  for (var start = 0; start < ids.length; start += 100) {
    final batch = ids.skip(start).take(100).toList();
    NostrEvent? cursor;
    for (var page = 0; ; page++) {
      if (page == 20) {
        throw StateError(
          'Activity updates exceed the read budget. Retry Activity.',
        );
      }
      final found = await query([
        NostrFilter(
          kinds: kinds,
          tags: {'#e': batch},
          limit: 500,
          until: cursor?.createdAt,
          extensions: {if (cursor != null) 'before_id': cursor.id},
        ),
      ]);
      if (found.isEmpty) break;
      found.sort((a, b) {
        final time = b.createdAt.compareTo(a.createdAt);
        return time != 0 ? time : a.id.compareTo(b.id);
      });
      if (cursor != null &&
          found.any(
            (e) =>
                e.createdAt > cursor!.createdAt ||
                (e.createdAt == cursor.createdAt &&
                    e.id.compareTo(cursor.id) <= 0),
          )) {
        throw StateError('Activity updates did not advance. Retry Activity.');
      }
      result.addAll(found);
      if (result.length > 4096) {
        throw StateError(
          'Activity updates exceed the evidence budget. Refresh Activity.',
        );
      }
      cursor = found.last;
    }
  }
  return result;
}

int _newest(NostrEvent a, NostrEvent b) {
  final time = b.createdAt.compareTo(a.createdAt);
  return time != 0 ? time : a.id.compareTo(b.id);
}

bool _matches(NostrFilter filter, NostrEvent event) =>
    filter.kinds.contains(event.kind) &&
    filter.tags.entries.every(
      (entry) => event.tags.any(
        (tag) =>
            tag.length > 1 &&
            tag[0] == entry.key.substring(1) &&
            entry.value.contains(tag[1]),
      ),
    );

/// Keep a bounded newest window, preserving one entry per conversation before
/// filling the remaining slots so busy channels do not evict sparse DMs.
void _trimEvidence(Map<String, NostrEvent> events) {
  if (events.length <= 4096) return;
  final ordered = events.values.toList()..sort(_newest);
  final channels = <String>{};
  final kept = <String, NostrEvent>{};
  for (final event in ordered) {
    final channel = event.channelId;
    if (channel != null && channels.add(channel)) kept[event.id] = event;
    if (kept.length == 4096) break;
  }
  for (final event in ordered) {
    if (kept.length == 4096) break;
    kept[event.id] = event;
  }
  events
    ..clear()
    ..addAll(kept);
}

List<NostrFilter> _coldFilters(List<String> ids) => [
  for (var start = 0; start < ids.length; start += kMaxExplicitChannelValues)
    NostrFilter(
      kinds: const [...activityMessageKinds, 7],
      tags: {'#h': ids.skip(start).take(kMaxExplicitChannelValues).toList()},
      limit: 500,
    ),
];
