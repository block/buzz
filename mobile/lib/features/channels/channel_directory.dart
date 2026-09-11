part of 'channels_provider.dart';

const _channelDirectoryPageSize = 500;
const _maxChannelDirectoryPages = 100;

/// Describes whether the open-channel directory is ready to browse.
enum ChannelDirectoryLoadStatus {
  /// No directory request has completed for the active identity and relay.
  idle,

  /// A directory request is currently in flight.
  loading,

  /// The directory request completed, including when it returned no channels.
  loaded,

  /// The most recent directory request could not complete.
  error,
}

/// Directory loading state scoped to one relay and signing identity.
class ChannelDirectoryLoadState {
  /// Relay-and-identity scope that produced [status].
  final String? scope;

  /// Current loading status for [scope].
  final ChannelDirectoryLoadStatus status;

  /// Creates directory loading state.
  const ChannelDirectoryLoadState({required this.scope, required this.status});

  /// Initial state before any directory request has started.
  const ChannelDirectoryLoadState.idle()
    : scope = null,
      status = ChannelDirectoryLoadStatus.idle;
}

/// Returns the stable directory scope for a relay and signing identity.
String channelDirectoryScope(String relayBaseUrl, String? pubkey) =>
    '$relayBaseUrl:${pubkey?.toLowerCase() ?? ''}';

/// Owns the independently observable channel-directory loading state.
class ChannelDirectoryLoadNotifier extends Notifier<ChannelDirectoryLoadState> {
  @override
  ChannelDirectoryLoadState build() => const ChannelDirectoryLoadState.idle();

  /// Whether [scope] currently owns an in-flight directory request.
  bool isLoading(String scope) =>
      state.scope == scope &&
      state.status == ChannelDirectoryLoadStatus.loading;

  /// Marks the directory as loading.
  void markLoading(String scope) => state = ChannelDirectoryLoadState(
    scope: scope,
    status: ChannelDirectoryLoadStatus.loading,
  );

  /// Marks the directory as eligible for a fresh request.
  void markIdle(String scope) => state = ChannelDirectoryLoadState(
    scope: scope,
    status: ChannelDirectoryLoadStatus.idle,
  );

  /// Marks the directory as successfully loaded.
  void markLoaded(String scope) => state = ChannelDirectoryLoadState(
    scope: scope,
    status: ChannelDirectoryLoadStatus.loaded,
  );

  /// Marks the directory request as unsuccessful.
  void markError(String scope) => state = ChannelDirectoryLoadState(
    scope: scope,
    status: ChannelDirectoryLoadStatus.error,
  );
}

/// Loading state for open-channel discovery, separate from membership loading.
final channelDirectoryLoadStatusProvider =
    NotifierProvider<ChannelDirectoryLoadNotifier, ChannelDirectoryLoadState>(
      ChannelDirectoryLoadNotifier.new,
    );

Future<List<NostrEvent>> _fetchChannelMemberships(
  RelaySessionNotifier session,
  String pubkey,
) => _fetchPaginatedChannelEvents(
  session,
  kind: 39002,
  tags: {
    '#p': [pubkey],
  },
  operation: 'Channel memberships',
);

Future<List<NostrEvent>> _fetchChannelDirectoryMetas(
  RelaySessionNotifier session,
) => _fetchPaginatedChannelEvents(
  session,
  kind: 39000,
  operation: 'Channel directory',
);

/// Thrown when a channel-list request is retired before it settles.
///
/// Callers must treat this as "write nothing": a newer request or scope now
/// owns the installed list and its related cache, subscription, and load state.
class _StaleChannelRefresh implements Exception {
  const _StaleChannelRefresh();

  @override
  String toString() =>
      'Channel refresh retired by a newer request or scope change';
}

/// Carries one channel-list refresh's request ownership across every await.
///
/// This token is captured once at the start of every ordinary, directory, and
/// reconnect refresh. It is re-checked after each relay await, so an older
/// request cannot regain ownership by reaching subscription setup last.
class _ChannelRefreshFence {
  /// Relay-and-identity scope that started the refresh.
  final String scope;

  final _ChannelRefreshCoordinator _coordinator;
  final int _generation;

  _ChannelRefreshFence(this._coordinator, this.scope, this._generation);

  /// Whether the refresh still owns the active scope and generation.
  bool get isCurrent =>
      _generation == _coordinator.generation &&
      scope == _coordinator.currentScope();

  /// Throws [_StaleChannelRefresh] once this refresh has been retired.
  ///
  /// Call after every await and immediately before every write to metadata,
  /// cache, load status, subscriptions or provider state.
  void ensureCurrent() {
    if (!isCurrent) throw const _StaleChannelRefresh();
  }
}

/// Whether a detached unread catch-up has been superseded, so it writes nothing.
///
/// The refresh fence is acquired before the first relay await, which makes this
/// request-ordered rather than subscription-completion-ordered. The subscription
/// generation remains a second lifecycle check for disconnect and disposal.
///
/// A retired catch-up returns rather than throwing [_StaleChannelRefresh]:
/// nothing awaits it, so a throw would only surface as an unhandled error.
///
/// An extension in this part file rather than a method on the notifier because
/// `channels_provider.dart` sits against the repository-wide 1200-line file
/// ceiling enforced by `just file-size-check`.
extension _CatchUpFencing on ChannelsNotifier {
  bool _isCatchUpRetired(
    _ChannelRefreshFence fence,
    int subscriptionGeneration,
  ) => !fence.isCurrent || subscriptionGeneration != _subscriptionVersion;
}

/// Awaits [future], then rejects the result if the refresh was retired.
///
/// One helper keeps every await site on the fenced path identical, so a new
/// await cannot be added without deciding whether it needs the fence.
///
/// The error path is fenced too. Without it a retired refresh that fails would
/// surface an ordinary exception, and `retryDirectory` would treat it as a
/// failure of the current scope: it would mark the wrong scope's status and
/// reinstall the channel list it captured before the switch.
Future<T> _fenced<T>(_ChannelRefreshFence fence, Future<T> future) async {
  final T value;
  try {
    value = await future;
  } catch (_) {
    if (!fence.isCurrent) throw const _StaleChannelRefresh();
    rethrow;
  }
  fence.ensureCurrent();
  return value;
}

/// Resolves display labels for the other participants in every DM meta.
///
/// The relay stores DM channels with the literal name "DM", and the pure-Nostr
/// architecture puts name resolution in the client. So collect the non-self
/// participant pubkeys across all DM metas and batch-fetch their kind:0
/// profiles in one round-trip. Returns lowercase pubkey to label.
///
/// Lives in this part file because `channels_provider.dart` sits against the
/// repository-wide 1200-line file ceiling enforced by `just file-size-check`.
Future<Map<String, String>> _resolveDmDisplayNames(
  RelaySessionNotifier session,
  _ChannelRefreshFence fence,
  Iterable<NostrEvent> dedupedMetas,
  String myPk,
) async {
  final dmParticipants = <String>{};
  final myPkLower = myPk.toLowerCase();
  for (final event in dedupedMetas) {
    final data = ChannelData.fromEvent(event);
    if (data.channelType != 'dm') continue;
    for (final pk in data.participantPubkeys) {
      final lower = pk.toLowerCase();
      if (lower != myPkLower) dmParticipants.add(lower);
    }
  }
  if (dmParticipants.isEmpty) return const {};

  final profileEvents = await _fenced(
    fence,
    session.fetchHistory(NostrFilters.profilesBatch(dmParticipants.toList())),
  );
  final displayNames = <String, String>{};
  for (final event in profileEvents) {
    if (event.kind != 0) continue;
    final profile = ProfileData.fromEvent(event);
    final label = profile.displayName?.trim().isNotEmpty == true
        ? profile.displayName!.trim()
        : profile.nip05?.trim().isNotEmpty == true
        ? profile.nip05!.trim()
        : shortPubkey(profile.pubkey);
    displayNames[profile.pubkey.toLowerCase()] = label;
  }
  return displayNames;
}

Future<Set<String>> _fetchHiddenDmIds(
  RelaySessionNotifier session,
  String myPk,
) async {
  try {
    final events = await session.fetchHistory(NostrFilters.hiddenDms(myPk));
    if (events.isEmpty) return const {};
    NostrEvent latest = events.first;
    for (final event in events.skip(1)) {
      if (event.createdAt > latest.createdAt) latest = event;
    }
    return {
      for (final tag in latest.tags)
        if (tag.length >= 2 && tag[0] == 'h') tag[1],
    };
  } catch (_) {
    return const {};
  }
}

Future<List<NostrEvent>> _fetchHuddleStarts(
  RelaySessionNotifier session,
  List<String> parentChannelIds,
) async {
  if (parentChannelIds.isEmpty) return const [];
  try {
    final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
    return await session.fetchHistory(
      NostrFilter(
        kinds: const [EventKind.huddleStarted],
        tags: {'#h': parentChannelIds},
        since: now - const Duration(hours: 2).inSeconds,
        limit: 500,
      ),
    );
  } catch (error) {
    debugPrint(
      '[ChannelsNotifier] Huddle backing-channel query failed: $error',
    );
    return const [];
  }
}

/// Counts distinct `p`-tagged members per channel from kind:39002 events.
///
/// Lives in this part file to keep `channels_provider.dart` under the
/// repository-wide 1200-line ceiling enforced by `just file-size-check`.
Map<String, int> _memberCountsByChannelId(Iterable<NostrEvent> memberEvents) {
  final memberCounts = <String, int>{};
  for (final event in memberEvents) {
    final channelId = event.getTagValue('d');
    if (channelId == null) continue;
    final pTags = <String>{};
    for (final tag in event.tags) {
      if (tag.isNotEmpty && tag[0] == 'p' && tag.length > 1) {
        pTags.add(tag[1].toLowerCase());
      }
    }
    memberCounts[channelId] = pTags.length;
  }
  return memberCounts;
}

/// Loads the open-channel directory fenced to the scope that requested it.
///
/// A community or identity switch changes the scope, and a newer request bumps
/// the generation. Either one retires an in-flight request, so a delayed
/// response can never populate the current community's state. This is the
/// tenant boundary described in VISION.md: isolation is the boundary, not a
/// filter, so a retired response is discarded rather than merged.
///
/// Lives in this part file because `channels_provider.dart` sits against the
/// repository-wide 1200-line file ceiling enforced by `just file-size-check`.
class _ChannelRefreshCoordinator {
  /// Resolves the relay-and-identity scope that is active right now.
  final String Function() currentScope;

  /// Owns the externally observable directory load status.
  final ChannelDirectoryLoadNotifier Function() loadStatus;

  int _generation = 0;

  /// Generation of the most recently issued or retired request.
  int get generation => _generation;

  _ChannelRefreshCoordinator({
    required this.currentScope,
    required this.loadStatus,
  });

  /// Binds the fence to a notifier's [Ref] so the provider needs one line.
  ///
  /// Both closures read rather than watch: the fence asks what the scope is
  /// right now, and must not make the notifier depend on it.
  factory _ChannelRefreshCoordinator.forRef(Ref ref) =>
      _ChannelRefreshCoordinator(
        currentScope: () => channelDirectoryScope(
          ref.read(relayConfigProvider).baseUrl,
          ref.read(myPubkeyProvider),
        ),
        loadStatus: () => ref.read(channelDirectoryLoadStatusProvider.notifier),
      );

  /// Retires any in-flight request without starting a new one.
  ///
  /// Called when the relay or identity changes so a response already on the
  /// wire cannot be written into the new scope.
  void retireInFlight() => _generation++;

  /// Starts a fenced refresh without issuing the directory query.
  ///
  /// Used by callers that must carry the scope across later awaits even when
  /// they do not refresh discovery, so a membership-only refresh cannot install
  /// an old scope's list either.
  _ChannelRefreshFence beginRefresh({required bool fetchesDirectory}) {
    final scope = currentScope();
    final generation = ++_generation;
    if (!fetchesDirectory) {
      final status = loadStatus();
      if (status.isLoading(scope)) {
        // This refresh takes ownership of the shared generation, so the
        // in-flight directory response will be discarded. The mounted Browse
        // sheet cannot restart an idle request on its own: it only starts a
        // load when it mounts and deliberately renders idle as its initial
        // spinner. Settle the displaced load to a retryable terminal state so
        // the sheet never waits forever for a response that may no longer
        // write results.
        status.markError(scope);
      }
    }
    return _ChannelRefreshFence(this, scope, generation);
  }

  /// Fetches the directory under [fence], or throws if the fence is retired.
  ///
  /// Returns null when the request failed inside the current scope, which means
  /// "retain the cached discovery". The fence is re-checked after the await and
  /// before every write, on both the success and the failure path.
  Future<List<NostrEvent>?> loadDirectory(
    RelaySessionNotifier session,
    _ChannelRefreshFence fence,
  ) async {
    loadStatus().markLoading(fence.scope);
    final List<NostrEvent> metas;
    try {
      metas = await _fetchChannelDirectoryMetas(session);
    } catch (error, stackTrace) {
      fence.ensureCurrent();
      loadStatus().markError(fence.scope);
      debugPrint(
        '[ChannelsNotifier] channel directory refresh failed; retaining '
        'cached discovery: $error\n$stackTrace',
      );
      return null;
    }
    fence.ensureCurrent();
    loadStatus().markLoaded(fence.scope);
    return metas;
  }
}

Future<List<NostrEvent>> _fetchPaginatedChannelEvents(
  RelaySessionNotifier session, {
  required int kind,
  required String operation,
  Map<String, List<String>> tags = const {},
}) async {
  final events = <NostrEvent>[];
  final seenEventIds = <String>{};
  int? until;
  String? beforeId;
  for (var pageIndex = 0; pageIndex < _maxChannelDirectoryPages; pageIndex++) {
    final page = await session.queryRelay([
      NostrFilter(
        kinds: [kind],
        tags: tags,
        limit: _channelDirectoryPageSize,
        until: until,
        extensions: {'before_id': ?beforeId},
      ),
    ]);
    if (page.isEmpty) break;
    var madeProgress = false;
    for (final event in page) {
      if (seenEventIds.add(event.id)) {
        events.add(event);
        madeProgress = true;
      }
    }
    if (!madeProgress) break;

    final last = page.last;
    until = last.createdAt;
    beforeId = last.id;
    if (pageIndex == _maxChannelDirectoryPages - 1) {
      throw StateError('$operation exceeded $_maxChannelDirectoryPages pages');
    }
  }
  return events;
}

/// Thread-interest and unread helpers shared by [ChannelsNotifier].
///
/// Lives in this part file because `channels_provider.dart` sits against the
/// repository-wide 1200-line file ceiling enforced by `just file-size-check`.
String? _observedUnreadRootId(NostrEvent event) =>
    _isBroadcastReply(event) ? null : event.threadReference.rootId;

bool _isBroadcastReply(NostrEvent event) => event.tags.any(
  (tag) => tag.length >= 2 && tag[0] == 'broadcast' && tag[1] == '1',
);

Set<String> _readRootIdSet(String? raw) {
  if (raw == null || raw.isEmpty) return {};
  try {
    final decoded = jsonDecode(raw);
    if (decoded is! List) return {};
    return {
      for (final value in decoded)
        if (value is String) value,
    };
  } catch (_) {
    return {};
  }
}

String _encodeRootIdSet(Set<String> values) => jsonEncode(values.toList());

/// Records one observed unread event for a channel's badge state.
///
/// An extension in this part file rather than a method on the notifier because
/// `channels_provider.dart` sits against the repository-wide 1200-line file
/// ceiling enforced by `just file-size-check`. Private members stay reachable:
/// a part shares its parent's library.
extension _ObservedUnreadRecording on ChannelsNotifier {
  void _recordUnreadEvent(Channel channel, NostrEvent event, String myPk) {
    final isThreadedReply =
        event.threadReference.parentId != null && !_isBroadcastReply(event);
    final isHighPriority =
        channel.isDm || isHighPriorityEvent(event.tags, myPk);
    recordObservedUnreadEvent(
      _observedUnreadEventsByChannel,
      channel.id,
      makeObservedUnreadEvent(
        id: event.id,
        createdAt: event.createdAt,
        rootId: _observedUnreadRootId(event),
        highPriority: isHighPriority,
        channelType: channel.channelType,
        isThreadedReply: isThreadedReply,
      ),
      _unreadCatchUpLimit,
    );

    final current = _latestObservedByChannel[channel.id] ?? 0;
    if (event.createdAt > current) {
      _latestObservedByChannel[channel.id] = event.createdAt;
    }
  }
}
