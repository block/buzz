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
  final Map<String, NostrEvent> rosters;

  /// The relay's clock, in unix seconds, when the read's removal check ran.
  final int asOf;
}

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
      final removalsRequest = _fetchMembershipRemovals(session, relay, since);
      final changedRequest = _fetchChannelMemberships(
        session,
        myPk,
        since: since,
      );
      await Future.wait(eagerError: true, [removalsRequest, changedRequest]);
      final removals = await removalsRequest;
      if (removals != null) {
        final rosters = _mergeRosters(
          cached.rosters,
          await changedRequest,
          removals.removedAt,
        );
        return (
          rosters: rosters.values.toList(),
          cache: _MembershipCache(fence.scope, rosters, removals.asOf),
        );
      }
    }
    // The removed read anchors the next read to the relay's clock. Its
    // answer is otherwise unused: the full read is authoritative. A failed
    // anchor only costs the next read its shortcut.
    final anchorRequest = _fetchMembershipRemovals(
      session,
      relay,
      DateTime.now().millisecondsSinceEpoch ~/ 1000,
    ).catchError((Object _) => null);
    final fullRequest = _fetchChannelMemberships(session, myPk);
    await Future.wait(eagerError: true, [anchorRequest, fullRequest]);
    final anchor = await anchorRequest;
    final rosters = await fullRequest;
    return (
      rosters: rosters,
      cache: anchor == null
          ? null
          : _MembershipCache(
              fence.scope,
              _mergeRosters(const {}, rosters, const {}),
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

/// Applies [changed] rosters over [cached] ones by channel id, newest first,
/// then drops each channel whose roster is no newer than its removal.
///
/// A roster newer than the removal means the reader was added back.
Map<String, NostrEvent> _mergeRosters(
  Map<String, NostrEvent> cached,
  List<NostrEvent> changed,
  Map<String, int> removedAt,
) => {
  for (final roster in _latestChannelEvents([
    ...cached.values,
    ...changed,
  ], kind: 39002))
    if (roster.getTagValue('d') case final id?)
      if (roster.createdAt > (removedAt[id] ?? -1)) id: roster,
};
