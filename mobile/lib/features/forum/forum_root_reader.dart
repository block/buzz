import 'dart:convert';

import '../../shared/relay/relay.dart';
import '../channels/channel_window.dart';
import '../channels/thread_window.dart';
import 'forum_ancestry.dart';

/// Pages forum posts using the channel window protocol (NIP-CW `top_level`).
/// Falls back to bounded standard-filter scans when channel windows are
/// unavailable or unusable, or the relay URL does not use HTTPS.
/// Once selected, fallback remains active for this reader's lifetime.
class ForumRootReader {
  final RelaySessionNotifier session;
  final String channelId;
  final String relayUrl;
  final void Function() checkActive;
  bool _legacy = false;
  Object? _pageError;
  Object? _auxError;

  /// An unproved standard-filter boundary or auxiliary closure stays visible.
  Object? get incompleteError => _pageError ?? _auxError;

  ForumRootReader(
    this.session,
    this.channelId,
    this.relayUrl,
    this.checkActive,
  );

  /// Reads one root page. Legacy scanning is bounded independently of how
  /// many older pages the user has loaded; it never declares a capped scan EOF.
  Future<ChannelWindowPage> fetch(ChannelPageCursor? cursor) async {
    checkActive();
    _pageError = null;
    // NIP-CW authenticated-transport profile. Plain HTTP uses verified events.
    if (!_legacy && Uri.parse(relayUrl).scheme == 'https') {
      try {
        checkActive();
        final events = await session.queryRelay([
          NostrFilters.forumPosts(
            channelId,
            until: cursor?.createdAt,
            beforeId: cursor?.eventId,
          ),
        ]);
        checkActive();
        if (events.any(
          (event) =>
              NostrFilters.forumRootKinds.contains(event.kind) &&
              event.channelId != channelId,
        )) {
          throw const FormatException('Forum row belongs to another channel');
        }
        try {
          _validateBounds(events, cursor);
        } catch (_) {
          // NIP-CW §Degradation: discard unusable overlays and reissue a
          // clean standard filter; never reuse these untrusted rows.
          _legacy = true;
        }
        if (!_legacy) {
          // A bad summary or row is an error, not a capability downgrade.
          return parseChannelWindowResponse(
            events,
            channelId,
            cursor,
            contentKinds: NostrFilters.forumRootKinds,
          );
        }
      } on RelayException catch (error) {
        checkActive();
        if (![400, 404, 405, 422].contains(error.statusCode)) rethrow;
        _legacy = true;
      }
    } else {
      _legacy = true;
    }
    return _fetchLegacy(cursor);
  }

  void _validateBounds(List<NostrEvent> events, ChannelPageCursor? cursor) {
    final bounds = events.where(
      (event) => event.kind == EventKind.channelWindowBounds,
    );
    final suffix = cursor == null
        ? 'head'
        : '${cursor.createdAt}:${cursor.eventId}';
    if (bounds.length != 1 ||
        bounds.single.getTagValue('d') != '$channelId:$suffix') {
      throw const FormatException('Invalid forum channel bounds binding');
    }
    final content = jsonDecode(bounds.single.content);
    if (content is! Map<String, dynamic> || content['has_more'] is! bool) {
      throw const FormatException('Invalid forum channel bounds content');
    }
    final next = parseWindowCursor(content['next_cursor']);
    if (content['has_more'] != (next != null)) {
      throw const FormatException('Invalid forum channel continuation');
    }
  }

  Future<ChannelWindowPage> _fetchLegacy(ChannelPageCursor? start) async {
    var scan = start;
    final roots = <NostrEvent>[];
    var exhausted = false;
    for (var page = 0; page < 20; page++) {
      checkActive();
      final events = await session.queryRelay([
        NostrFilter(
          kinds: NostrFilters.forumRootKinds,
          tags: {
            '#h': [channelId],
          },
          limit: 200,
          until: scan?.createdAt,
        ),
      ]);
      checkActive();
      await verifyForumEvents(events);
      checkActive();
      if (events.any(
        (event) =>
            event.channelId != channelId ||
            !NostrFilters.forumRootKinds.contains(event.kind),
      )) {
        throw const FormatException('Invalid legacy forum event scope');
      }
      // Inclusive timestamp reads retain the boundary second. Locally order
      // and deduplicate it using the same composite order as window mode.
      final ordered = events.toList()..sort(_order);
      final fresh = ordered
          .where(
            (event) => scan == null || isOlderThanChannelCursor(event, scan),
          )
          .toList();
      if (fresh.isEmpty) {
        if (events.isEmpty) {
          exhausted = true;
          break;
        }
        final error = StateError(
          'Legacy relay cannot continue this timestamp; history is incomplete',
        );
        if (roots.isEmpty) throw error;
        // Even a short response may be clamped inside this second. Publish
        // roots already found, but never call an ambiguous boundary exhausted.
        _pageError = error;
        break;
      }
      for (final event in fresh) {
        scan = ChannelPageCursor(createdAt: event.createdAt, eventId: event.id);
        if (event.threadReference.parentId == null) roots.add(event);
        if (roots.length == 50) break;
      }
      if (roots.length == 50) {
        break;
      }
    }
    if (!exhausted && scan == null) {
      throw StateError('Legacy forum scan made no progress');
    }
    if (exhausted && start == null && roots.isEmpty) {
      // The whole root range is empty, so no retained auxiliary gap matters.
      _auxError = null;
    }
    final aux = <NostrEvent>[];
    if (roots.isNotEmpty) {
      final first = await _aux({
        for (final event in roots) event.id: event,
      }, EventKind.channelAuxEventKinds);
      aux.addAll(first);
      if (first.isNotEmpty) {
        aux.addAll(
          await _aux(
            {
              for (final event in [...roots, ...first]) event.id: event,
            },
            const [EventKind.deletion, EventKind.nip29DeleteEvent],
            ids: first.map((event) => event.id).toList(),
          ),
        );
      }
    }
    return ChannelWindowPage(
      startCursor: start,
      rows: roots.map((event) => ChannelWindowRow(event: event)).toList(),
      aux: aux,
      nextCursor: exhausted ? null : scan,
      hasMore: !exhausted,
    );
  }

  Future<List<NostrEvent>> _aux(
    Map<String, NostrEvent> targets,
    List<int> kinds, {
    List<String>? ids,
  }) async {
    final collected = <String, NostrEvent>{};
    int? until;
    for (var page = 0; page < 20; page++) {
      checkActive();
      final events = await session.queryRelay([
        NostrFilter(
          kinds: kinds,
          tags: {'#e': ids ?? targets.keys.toList()},
          limit: 1000,
          until: until,
        ),
      ]);
      checkActive();
      if (events.isEmpty) return collected.values.toList();
      if (events.length >= 1000 ||
          events.any((event) => !kinds.contains(event.kind))) {
        throw StateError('Legacy forum auxiliary history is incomplete');
      }
      await verifyForumEvents(events);
      checkActive();
      _authorizeAux(events, targets);
      final fresh = events.where((event) => !collected.containsKey(event.id));
      if (fresh.isEmpty) break;
      collected.addAll({for (final event in fresh) event.id: event});
      if (collected.length >= 1000) {
        throw StateError('Legacy forum auxiliary history is incomplete');
      }
      final oldest = events
          .map((event) => event.createdAt)
          .reduce((a, b) => a < b ? a : b);
      if (until != null && oldest > until) {
        throw const FormatException('Invalid legacy auxiliary cursor scope');
      }
      until = oldest;
    }
    // An inclusive boundary that stops yielding new IDs may hide more events
    // in that second under a relay clamp. Keep the authorized effects fetched,
    // but expose incomplete closure rather than treating a short page as EOF.
    _auxError ??= StateError('Legacy forum auxiliary history is incomplete');
    return collected.values.toList();
  }

  void _authorizeAux(List<NostrEvent> events, Map<String, NostrEvent> targets) {
    // Plain #e reads provide no relay authorization guarantee. Never infer
    // moderator or agent-owner authority from an event's valid signature.
    for (final event in events) {
      final edit = event.kind == EventKind.streamMessageEdit;
      if (!edit &&
          event.kind != EventKind.deletion &&
          event.kind != EventKind.nip29DeleteEvent) {
        continue;
      }
      final references = event.tags
          .where((tag) => tag.length >= 2 && tag[0] == 'e')
          .map((tag) => tag[1]);
      for (final id
          in edit ? references.toList().reversed.take(1) : references) {
        if (targets[id]?.pubkey != event.pubkey) {
          throw StateError('Legacy auxiliary target author is not authorized');
        }
      }
    }
  }
}

int _order(NostrEvent a, NostrEvent b) {
  final time = b.createdAt.compareTo(a.createdAt);
  return time == 0 ? a.id.compareTo(b.id) : time;
}
