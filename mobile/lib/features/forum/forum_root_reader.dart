import '../../shared/relay/relay.dart';
import '../channels/channel_window.dart';
import 'forum_ancestry.dart';

/// Channel windows with NIP-CW's clean standard-filter degradation path.
class ForumRootReader {
  final RelaySessionNotifier session;
  final String channelId;
  final String relayUrl;
  bool _legacy = false;

  ForumRootReader(this.session, this.channelId, this.relayUrl);

  /// Reads one root page. Legacy scanning is bounded independently of how
  /// many older pages the user has loaded; it never declares a capped scan EOF.
  Future<ChannelWindowPage> fetch(ChannelPageCursor? cursor) async {
    // NIP-CW authenticated-transport profile. Plain HTTP uses verified events.
    if (!_legacy && Uri.parse(relayUrl).scheme == 'https') {
      try {
        final events = await session.queryRelay([
          NostrFilters.forumPosts(
            channelId,
            until: cursor?.createdAt,
            beforeId: cursor?.eventId,
          ),
        ]);
        try {
          final page = parseChannelWindowResponse(
            events,
            channelId,
            cursor,
            contentKinds: NostrFilters.forumRootKinds,
          );
          if (page.rows.any((row) => row.event.channelId != channelId)) {
            throw const FormatException('Forum row belongs to another channel');
          }
          return page;
        } catch (_) {
          // NIP-CW §Degradation: discard unusable overlays and reissue a
          // clean standard filter; never reuse these untrusted rows.
          _legacy = true;
        }
      } on RelayException catch (error) {
        if (![400, 404, 405, 422].contains(error.statusCode)) rethrow;
        _legacy = true;
      }
    } else {
      _legacy = true;
    }
    return _fetchLegacy(cursor);
  }

  Future<ChannelWindowPage> _fetchLegacy(ChannelPageCursor? start) async {
    var scan = start;
    final roots = <NostrEvent>[];
    var exhausted = false;
    for (var page = 0; page < 20; page++) {
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
      await verifyForumEvents(events);
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
      if (fresh.isEmpty && events.length >= 200) {
        throw StateError(
          'Legacy relay cannot continue this timestamp; history is incomplete',
        );
      }
      for (final event in fresh) {
        scan = ChannelPageCursor(createdAt: event.createdAt, eventId: event.id);
        if (event.threadReference.parentId == null) roots.add(event);
        if (roots.length == 50) break;
      }
      if (roots.length == 50) {
        exhausted = events.length < 200 && fresh.last.id == roots.last.id;
        break;
      }
      if (events.length < 200) {
        exhausted = true;
        break;
      }
    }
    if (!exhausted && scan == null) {
      throw StateError('Legacy forum scan made no progress');
    }
    final aux = <NostrEvent>[];
    if (roots.isNotEmpty) {
      final first = await _aux(
        roots.map((event) => event.id).toList(),
        EventKind.channelAuxEventKinds,
      );
      aux.addAll(first);
      if (first.isNotEmpty) {
        aux.addAll(
          await _aux(first.map((event) => event.id).toList(), const [
            EventKind.deletion,
            EventKind.nip29DeleteEvent,
          ]),
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

  Future<List<NostrEvent>> _aux(List<String> ids, List<int> kinds) async {
    final events = await session.queryRelay([
      NostrFilter(kinds: kinds, tags: {'#e': ids}, limit: 1000),
    ]);
    if (events.length >= 1000) {
      throw StateError('Legacy forum auxiliary history is incomplete');
    }
    await verifyForumEvents(events);
    return events;
  }
}

int _order(NostrEvent a, NostrEvent b) {
  final time = b.createdAt.compareTo(a.createdAt);
  return time == 0 ? a.id.compareTo(b.id) : time;
}
