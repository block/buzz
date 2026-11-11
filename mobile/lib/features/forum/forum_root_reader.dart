import 'dart:convert';

import '../../shared/relay/relay.dart';
import '../channels/channel_window.dart';
import '../channels/thread_window.dart';
import 'forum_ancestry.dart';
import 'forum_auxiliary.dart';
import 'forum_window.dart';

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
  bool _incomplete = false;

  /// Standard-filter continuations cannot prove that older roots exist.
  bool get usesLegacyFilters => _legacy;

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
    _incomplete = false;
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
        _incomplete = true;
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
    final aux = <NostrEvent>[];
    final auxiliaryTargets = <String, Set<String>>{};
    if (roots.isNotEmpty) {
      final first = await _aux({
        for (final event in roots) event.id: event,
      }, EventKind.channelAuxEventKinds);
      aux.addAll(first);
      final rootIds = roots.map((event) => event.id).toSet();
      for (final event in first) {
        if (_isDeletion(event)) {
          auxiliaryTargets[event.id] = forumKnownDeletionTargets(
            event,
            rootIds,
          );
        }
      }
      if (first.isNotEmpty) {
        final second = await _aux(
          {
            for (final event in [...roots, ...first]) event.id: event,
          },
          const [EventKind.deletion, EventKind.nip29DeleteEvent],
          ids: first.map((event) => event.id).toList(),
        );
        aux.addAll(second);
        final knownIds = {...rootIds, ...first.map((event) => event.id)};
        for (final event in second) {
          auxiliaryTargets[event.id] = forumKnownDeletionTargets(
            event,
            knownIds,
          );
        }
      }
    }
    return ForumWindowPage(
      startCursor: start,
      rows: roots.map((event) => ChannelWindowRow(event: event)).toList(),
      aux: aux,
      nextCursor: exhausted ? null : scan,
      hasMore: !exhausted,
      incompletePostIds: _incomplete
          ? roots.map((event) => event.id).toSet()
          : {},
      auxiliaryTargets: auxiliaryTargets,
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
      if (events.isEmpty) {
        return _authorizeAux(collected.values.toList(), targets);
      }
      if (events.length >= 1000 ||
          events.any((event) => !kinds.contains(event.kind))) {
        throw StateError('Legacy forum auxiliary history is incomplete');
      }
      await verifyForumEvents(events);
      checkActive();
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
    _incomplete = true;
    return _authorizeAux(collected.values.toList(), targets);
  }

  /// Keeps verified author effects; unproved authority leaves this page
  /// incomplete without discarding its readable posts. Plain #e reads cannot
  /// establish agent-owner or moderator authority from a signature alone.
  List<NostrEvent> _authorizeAux(
    List<NostrEvent> events,
    Map<String, NostrEvent> targets,
  ) => events.where((event) {
    if (!forumEditTargetsAgree(event) ||
        !forumAuxiliaryAuthorMatches(event, targets)) {
      _incomplete = true;
      return false;
    }
    return true;
  }).toList();
}

bool _isDeletion(NostrEvent event) =>
    event.kind == EventKind.deletion ||
    event.kind == EventKind.nip29DeleteEvent;

int _order(NostrEvent a, NostrEvent b) {
  final time = b.createdAt.compareTo(a.createdAt);
  return time == 0 ? a.id.compareTo(b.id) : time;
}
