part of 'channels_provider.dart';

/// The rosters from the last committed membership read, so the next read can
/// ask only for what changed.
///
/// Lives in this part file because `channels_provider.dart` sits against the
/// repository-wide 1200-line file ceiling enforced by `just file-size-check`.
class _MembershipCache {
  const _MembershipCache(this.scope, this.rosters, this.asOf);

  /// Relay-and-identity scope the rosters belong to.
  final String scope;

  /// The latest kind:39002 roster tagging the reader, by channel id.
  final Map<String, _CachedRoster> rosters;

  /// The relay's clock, in unix seconds, when the read's removal check ran.
  final int asOf;
}

/// A cached roster and the removal-clock time it is known current at.
///
/// The roster was read after a removed read answered with [currentAt], so
/// it outranks any removal at or before then. Removal times come from the
/// relay's database clock and roster `created_at` from its process clock, so
/// the two are never compared with each other.
typedef _CachedRoster = ({NostrEvent roster, int currentAt});

/// A membership read: the rosters to list, and the cache to keep once the
/// list built from them is installed. A null cache keeps none.
typedef _MembershipRead = ({List<NostrEvent> rosters, _MembershipCache? cache});

/// Channels the reader stopped being a member of, from
/// `{"membership":"removed","since":T}`: the relay's clock and, by channel
/// id, when the membership ended.
typedef _MembershipRemovals = ({int asOf, Map<String, int> removedAt});

/// How long a relay that refused the removed read is not asked again.
const _membershipRemovalsRetry = Duration(minutes: 10);

extension _MembershipDelta on ChannelsNotifier {
  /// Reads the reader's rosters, asking only for changes since the last
  /// committed read when the relay can report removed memberships.
  ///
  /// A `since` roster read returns every roster regenerated since then (joins,
  /// role changes, other members' changes) but not channels the reader left,
  /// was removed from, or that were deleted. The relay's removed read reports
  /// exactly those. When that read is refused (an older relay answers 403; a
  /// relay with too many changes, 413) or answers in an unexpected shape, the
  /// rosters are read in full.
  Future<_MembershipRead> _readMemberships(
    RelaySessionNotifier session,
    String myPk,
    _ChannelRefreshFence fence,
  ) async {
    final relay = _lifecycleRef.read(relayConfigProvider).baseUrl;
    final refused = _membershipRemovalsRefused;
    if (refused != null &&
        refused.relay == relay &&
        DateTime.now().isBefore(refused.until)) {
      return (
        rosters: await _fetchChannelMemberships(session, myPk),
        cache: null,
      );
    }
    final cached = _membershipCache;
    if (cached != null && cached.scope == fence.scope) {
      final since = max(0, cached.asOf - relayLiveWindowSkewSeconds);
      // Removals first: a roster read after them is current, so it outranks
      // every removal they report.
      final removals = await _fetchMembershipRemovals(session, relay, since);
      if (removals != null) {
        final rosters = _mergeRosters(
          cached.rosters,
          await _fetchChannelMemberships(session, myPk, since: since),
          removals,
        );
        return (
          rosters: [for (final entry in rosters.values) entry.roster],
          cache: _MembershipCache(fence.scope, rosters, removals.asOf),
        );
      }
    }
    // The removed read anchors the next read to the relay's clock. Its
    // answer is otherwise unused: the full read after it is authoritative.
    // A failed anchor only costs the next read its shortcut.
    final anchor = await _fetchMembershipRemovals(
      session,
      relay,
      DateTime.now().millisecondsSinceEpoch ~/ 1000,
    ).catchError((Object _) => null);
    final rosters = await _fetchChannelMemberships(session, myPk);
    return (
      rosters: rosters,
      cache: anchor == null
          ? null
          : _MembershipCache(
              fence.scope,
              _mergeRosters(const {}, rosters, anchor),
              anchor.asOf,
            ),
    );
  }

  /// The removed read since [since], or null when [relay] refuses it or
  /// answers in an unexpected shape. Transport failures and retired sessions
  /// still throw, as they would for the roster read.
  ///
  /// A refusal other than 413 (too many changes) or 429 (rate limited), and
  /// an unreadable answer, mean the relay does not support the read, so it is
  /// not asked again for [_membershipRemovalsRetry].
  Future<_MembershipRemovals?> _fetchMembershipRemovals(
    RelaySessionNotifier session,
    String relay,
    int since,
  ) async {
    try {
      return _parseMembershipRemovals(
        await session.queryRelayJson([
          {'membership': 'removed', 'since': since},
        ]),
      );
    } on RelayException catch (error) {
      debugPrint('[ChannelsNotifier] removed memberships refused: $error');
      final status = error.statusCode;
      if (status != 413 && status != 429 && status < 500) {
        _refuseMembershipRemovals(relay);
      }
      return null;
    } on FormatException catch (error) {
      debugPrint('[ChannelsNotifier] removed memberships unreadable: $error');
      _refuseMembershipRemovals(relay);
      return null;
    }
  }

  void _refuseMembershipRemovals(String relay) => _membershipRemovalsRefused = (
    relay: relay,
    until: DateTime.now().add(_membershipRemovalsRetry),
  );
}

_MembershipRemovals _parseMembershipRemovals(Object? decoded) {
  const bad = FormatException('Unexpected removed-memberships answer');
  if (decoded is! Map<String, dynamic>) throw bad;
  final asOf = decoded['asOf'];
  final removed = decoded['removed'];
  if (asOf is! int || removed is! List) throw bad;
  // A relay clock far ahead would start every later read past changes made
  // once it is corrected.
  final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
  if (asOf > now + relayLiveWindowSkewSeconds) {
    throw const FormatException('Removed-memberships clock is in the future');
  }
  final removedAt = <String, int>{};
  for (final row in removed) {
    if (row is! Map<String, dynamic>) throw bad;
    final channelId = row['channelId'];
    final at = row['at'];
    if (channelId is! String || at is! int) throw bad;
    removedAt[channelId] = max(removedAt[channelId] ?? at, at);
  }
  return (asOf: asOf, removedAt: removedAt);
}

/// Applies [changed] rosters, read after [removals], over [cached] ones by
/// channel id, then drops each cached channel removed since its roster was
/// known current.
///
/// A changed roster was read after every removal reported, so it is kept:
/// the reader was added back.
Map<String, _CachedRoster> _mergeRosters(
  Map<String, _CachedRoster> cached,
  List<NostrEvent> changed,
  _MembershipRemovals removals,
) {
  // Both times are whole seconds, so a removal in the same second as
  // [_CachedRoster.currentAt] may have come after the roster was read. It is
  // taken as a removal: a re-added roster is returned again while the
  // removal is still in the read window, and then outranks it.
  final rosters = {
    for (final MapEntry(:key, :value) in cached.entries)
      if (value.currentAt > (removals.removedAt[key] ?? -1)) key: value,
  };
  for (final roster in _latestChannelEvents(changed, kind: 39002)) {
    final id = roster.getTagValue('d');
    if (id == null) continue;
    final previous = rosters[id]?.roster;
    rosters[id] = (
      roster: previous != null && previous.createdAt > roster.createdAt
          ? previous
          : roster,
      currentAt: removals.asOf,
    );
  }
  return rosters;
}
