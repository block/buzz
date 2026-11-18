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

  /// Whether the last directory request for [scope] failed.
  bool hasFailed(String scope) =>
      state.scope == scope && state.status == ChannelDirectoryLoadStatus.error;

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
  String pubkey, {
  void Function()? ensureCurrent,
  bool strong = false,
}) => _fetchPaginatedChannelEvents(
  session,
  kind: 39002,
  tags: {
    '#p': [pubkey],
  },
  operation: 'Channel memberships',
  ensureCurrent: ensureCurrent,
  strong: strong,
);

Future<List<NostrEvent>> _fetchChannelDirectoryMetas(
  RelaySessionNotifier session, {
  int? since,
}) => _fetchPaginatedChannelEvents(
  session,
  kind: 39000,
  operation: 'Channel directory',
  since: since,
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
/// What a directory load checks before each write: the scope it serves.
abstract interface class _DirectoryLoadFence {
  /// Relay-and-identity scope that started the load.
  String get scope;

  /// Throws [_StaleChannelRefresh] once the load must write nothing.
  void ensureCurrent();
}

/// Fences a directory-only load to its scope alone.
///
/// Unlike [_ChannelRefreshFence] it holds no refresh generation: a directory
/// read does not depend on memberships, so it neither retires an in-flight
/// list refresh nor is retired by one. A community or identity switch, or
/// disposal, still retires it, so its response never reaches another scope.
class _DirectoryOnlyFence implements _DirectoryLoadFence {
  @override
  final String scope;

  final _ChannelRefreshCoordinator _coordinator;

  _DirectoryOnlyFence(this._coordinator, this.scope);

  @override
  void ensureCurrent() {
    if (!_coordinator.isMounted() || scope != _coordinator.currentScope()) {
      throw const _StaleChannelRefresh();
    }
  }
}

class _ChannelRefreshFence implements _DirectoryLoadFence {
  /// Relay-and-identity scope that started the refresh.
  @override
  final String scope;

  final _ChannelRefreshCoordinator _coordinator;
  final int _generation;

  _ChannelRefreshFence(this._coordinator, this.scope, this._generation);

  /// Whether the refresh still owns the active scope and generation.
  bool get isCurrent =>
      _coordinator.isMounted() &&
      _generation == _coordinator.generation &&
      scope == _coordinator.currentScope();

  /// Throws [_StaleChannelRefresh] once this refresh has been retired.
  ///
  /// Call after every await and immediately before every write to metadata,
  /// cache, load status, subscriptions or provider state.
  @override
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
  Ref ref,
  _ChannelRefreshFence fence,
  Iterable<ChannelData> channelMetas,
  String myPk, {
  required bool refreshCached,
}) async {
  final dmParticipants = <String>{};
  final myPkLower = myPk.toLowerCase();
  for (final data in channelMetas) {
    if (data.channelType != 'dm') continue;
    for (final pk in data.participantPubkeys) {
      final lower = pk.toLowerCase();
      if (lower != myPkLower) dmParticipants.add(lower);
    }
  }
  if (dmParticipants.isEmpty) return const {};

  final profiles = await _fenced(
    fence,
    ref
        .read(userCacheProvider.notifier)
        .resolve(dmParticipants.toList(), refreshCached: refreshCached),
  );
  final displayNames = <String, String>{};
  for (final profile in profiles.values) {
    final label = profile.displayName?.trim().isNotEmpty == true
        ? profile.displayName!.trim()
        : profile.nip05Handle?.trim().isNotEmpty == true
        ? profile.nip05Handle!.trim()
        : shortPubkey(profile.pubkey);
    displayNames[profile.pubkey.toLowerCase()] = label;
  }
  return displayNames;
}

List<NostrEvent> _latestChannelEvents(
  Iterable<NostrEvent> events, {
  required int kind,
}) {
  final latestById = <String, NostrEvent>{};
  for (final event in events) {
    if (event.kind != kind) continue;
    final id = event.getTagValue('d');
    if (id == null) continue;
    final current = latestById[id];
    if (current == null ||
        event.createdAt > current.createdAt ||
        (event.createdAt == current.createdAt &&
            event.id.compareTo(current.id) < 0)) {
      latestById[id] = event;
    }
  }
  return latestById.values.toList();
}

/// The hidden DM ids, or null when the relay doesn't answer.
Future<Set<String>?> _fetchHiddenDmIds(
  RelaySessionNotifier session,
  String myPk,
) async {
  try {
    return hiddenDmIdsFromEvents(
      await session.fetchHistory(NostrFilters.hiddenDms(myPk)),
    );
  } catch (_) {
    return null;
  }
}

/// The DM ids hidden by the newest of the caller's NIP-DV snapshots.
Set<String> hiddenDmIdsFromEvents(List<NostrEvent> events) {
  if (events.isEmpty) return const {};
  NostrEvent latest = events.first;
  for (final event in events.skip(1)) {
    if (event.createdAt > latest.createdAt) latest = event;
  }
  return {
    for (final tag in latest.tags)
      if (tag.length >= 2 && tag[0] == 'h') tag[1],
  };
}

Future<List<NostrEvent>> _fetchHuddleStarts(
  RelaySessionNotifier session,
  List<String> parentChannelIds,
) async {
  if (parentChannelIds.isEmpty) return const [];
  try {
    final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
    // Each REQ stays within the relay's aggregate explicit-channel cap, with
    // the same bounded concurrency as channel history.
    final filters = [
      for (
        var start = 0;
        start < parentChannelIds.length;
        start += kMaxExplicitChannelValues
      )
        NostrFilter(
          kinds: const [EventKind.huddleStarted],
          tags: {
            '#h': parentChannelIds.sublist(
              start,
              min(start + kMaxExplicitChannelValues, parentChannelIds.length),
            ),
          },
          since: now - const Duration(hours: 2).inSeconds,
          limit: 500,
        ),
    ];
    final results = <List<NostrEvent>>[];
    for (
      var start = 0;
      start < filters.length;
      start += _historyRequestConcurrency
    ) {
      results.addAll(
        await Future.wait(
          filters
              .sublist(
                start,
                min(start + _historyRequestConcurrency, filters.length),
              )
              .map(session.fetchHistory),
        ),
      );
    }
    return [for (final events in results) ...events];
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
  /// Whether the owning provider can still read its current scope.
  final bool Function() isMounted;

  /// Resolves the relay-and-identity scope that is active right now.
  final String Function() currentScope;

  /// Owns the externally observable directory load status.
  final ChannelDirectoryLoadNotifier Function() loadStatus;

  /// The device clock, which dates when each directory read starts.
  final DateTime Function() clock;

  int _generation = 0;

  /// Generation of the most recently issued or retired request.
  int get generation => _generation;

  // Directory loads are ordered among themselves: only the newest one writes
  // the directory or its status. A directory-only load is not retired by a
  // list refresh, so [beginRefresh] must leave its loading status alone.
  int _directoryGeneration = 0;
  int? _directoryOnlyGeneration;

  /// For each directory load in flight, the channels dropped on open since it
  /// started. Its response predates the drop, so it must not bring them back.
  final _forgottenByLoad = <Set<String>>[];

  /// Records that opening [channelId] found the relay no longer serves it.
  void forget(String channelId) {
    for (final forgotten in _forgottenByLoad) {
      forgotten.add(channelId);
    }
  }

  /// The generation of the non-quiet load in flight, if any. A quiet load
  /// never displaces it: readers wait on its status.
  int? _loudGeneration;

  /// Starts a directory-only load for the active scope.
  _DirectoryOnlyFence beginDirectoryOnly() =>
      _DirectoryOnlyFence(this, currentScope());

  _ChannelRefreshCoordinator({
    required this.isMounted,
    required this.currentScope,
    required this.loadStatus,
    required this.clock,
  });

  /// Binds the fence to a notifier's [Ref] so the provider needs one line.
  ///
  /// The closures read rather than watch: the fence checks the current
  /// lifecycle and scope without making the notifier depend on them.
  factory _ChannelRefreshCoordinator.forRef(Ref ref) =>
      _ChannelRefreshCoordinator(
        isMounted: () => ref.mounted,
        currentScope: () => channelDirectoryScope(
          ref.read(relayConfigProvider).baseUrl,
          ref.read(myPubkeyProvider),
        ),
        loadStatus: () => ref.read(channelDirectoryLoadStatusProvider.notifier),
        clock: () => ref.read(channelDirectoryClockProvider)(),
      );

  /// Retires any in-flight request without starting a new one.
  ///
  /// Called when the relay or identity changes so a response already on the
  /// wire cannot be written into the new scope.
  void retireInFlight() => _generation++;

  /// Retires every in-flight request, directory-only loads included, for a
  /// relay or identity change. A directory-only load survives ordinary list
  /// refreshes, but not a scope change: its loading status must not look
  /// owned when the user returns to the old scope.
  void retireScope() {
    _generation++;
    _directoryGeneration++;
    _directoryOnlyGeneration = null;
  }

  /// Starts a fenced refresh without issuing the directory query.
  ///
  /// Used by callers that must carry the scope across later awaits even when
  /// they do not refresh discovery, so a membership-only refresh cannot install
  /// an old scope's list either.
  Future<_ChannelRefreshFence> beginRefresh({
    required bool fetchesDirectory,
  }) async {
    final scope = currentScope();
    final generation = ++_generation;
    final fence = _ChannelRefreshFence(this, scope, generation);
    // Acquire ownership synchronously, but let the provider finish building
    // before changing another provider's directory status.
    await Future<void>.value();
    fence.ensureCurrent();
    if (!fetchesDirectory) {
      final status = loadStatus();
      if (status.isLoading(scope) &&
          _directoryOnlyGeneration != _directoryGeneration) {
        // This refresh takes ownership of the shared generation, so the
        // in-flight directory response will be discarded. Mark it idle, not
        // error: nothing failed, so a caller may load it again, and search
        // retries a displaced load once instead of reporting a failure.
        status.markIdle(scope);
      }
    } else {
      final status = loadStatus();
      if (status.hasFailed(scope)) {
        // A new attempt replaces the last failure. The status turns loading
        // only after the membership reads, so a refresh that retires this
        // attempt earlier must not leave the old error to be read as its
        // result: idle lets search retry the displaced attempt.
        status.markIdle(scope);
      }
    }
    return fence;
  }

  /// Fetches the directory under [fence], or throws if the fence is retired.
  ///
  /// With [since], fetches only metadata changed at or after it: a catch-up
  /// for a directory already shown. A [quiet] load (a catch-up, or the daily
  /// full read behind a saved copy) leaves the load status alone, so search
  /// keeps showing the saved copy without a spinner; its failure is logged
  /// and retried the next time a reader opens.
  ///
  /// Returns the metadata with when the read started, or null when the
  /// request failed inside the current scope, which means
  /// "retain the cached discovery", when a quiet load would displace a
  /// non-quiet one in flight, or when a newer directory load started,
  /// which then owns the directory and its status. The fence is re-checked
  /// after the await and before every write, on both the success and the
  /// failure path.
  Future<_DirectoryRead?> loadDirectory(
    RelaySessionNotifier session,
    _DirectoryLoadFence fence, {
    int? since,
    bool quiet = false,
  }) async {
    if (quiet && _loudGeneration == _directoryGeneration) return null;
    final directoryGeneration = ++_directoryGeneration;
    _directoryOnlyGeneration = fence is _DirectoryOnlyFence
        ? directoryGeneration
        : null;
    if (!quiet) {
      _loudGeneration = directoryGeneration;
      loadStatus().markLoading(fence.scope);
    }
    final startedAt = clock();
    final forgotten = <String>{};
    _forgottenByLoad.add(forgotten);
    final List<NostrEvent> metas;
    try {
      metas = await _fetchChannelDirectoryMetas(session, since: since);
    } catch (error, stackTrace) {
      fence.ensureCurrent();
      if (directoryGeneration != _directoryGeneration) return null;
      if (!quiet) loadStatus().markError(fence.scope);
      debugPrint(
        '[ChannelsNotifier] channel directory refresh failed; retaining '
        'cached discovery: $error\n$stackTrace',
      );
      return null;
    } finally {
      _forgottenByLoad.remove(forgotten);
      if (_loudGeneration == directoryGeneration) _loudGeneration = null;
    }
    fence.ensureCurrent();
    if (directoryGeneration != _directoryGeneration) return null;
    loadStatus().markLoaded(fence.scope);
    return (
      metas: [
        for (final event in metas)
          if (!forgotten.contains(event.getTagValue('d'))) event,
      ],
      startedAt: startedAt,
    );
  }
}

/// A directory read's metadata, and when it started by the device clock.
typedef _DirectoryRead = ({List<NostrEvent> metas, DateTime startedAt});

Future<List<NostrEvent>> _fetchPaginatedChannelEvents(
  RelaySessionNotifier session, {
  required int kind,
  required String operation,
  Map<String, List<String>> tags = const {},
  void Function()? ensureCurrent,
  bool strong = false,
  int? since,
}) async {
  final events = <NostrEvent>[];
  final seenEventIds = <String>{};
  int? until;
  String? beforeId;
  for (var pageIndex = 0; pageIndex < _maxChannelDirectoryPages; pageIndex++) {
    ensureCurrent?.call();
    final page = await session.queryRelay([
      NostrFilter(
        kinds: [kind],
        tags: tags,
        limit: _channelDirectoryPageSize,
        since: since,
        until: until,
        extensions: {
          'before_id': ?beforeId,
          // Reads the relay's writer pool, so it sees this client's own
          // just-accepted writes that replicas may not have yet.
          if (strong) 'consistency': 'strong',
        },
      ),
    ]);
    ensureCurrent?.call();
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
  bool _recordUnreadEvent(
    Channel channel,
    NostrEvent event,
    String myPk, {
    bool countQuiet = false,
  }) {
    if (_unscopedUnreadDeletedIds.contains(event.id) ||
        _unreadDeletedIdsByChannel[channel.id]?.contains(event.id) == true) {
      return false;
    }
    if (!countQuiet && isQuietAgentMessage(event.tags)) return false;
    // A counted quiet event is only a backlog marker: it never earns attention
    // through its mentions or thread.
    final isThreadedReply =
        !countQuiet &&
        event.threadReference.parentId != null &&
        !_isBroadcastReply(event);
    final isHighPriority =
        channel.isDm || (!countQuiet && isHighPriorityEvent(event.tags, myPk));
    recordObservedUnreadEvent(
      _observedUnreadEventsByChannel,
      channel.id,
      makeObservedUnreadEvent(
        id: event.id,
        createdAt: event.createdAt,
        rootId: countQuiet ? null : _observedUnreadRootId(event),
        highPriority: isHighPriority,
        channelType: channel.channelType,
        isThreadedReply: isThreadedReply,
        threadReply: isThreadedReply && !channel.isDm ? event : null,
      ),
      _unreadCatchUpLimit,
    );

    final current = _latestObservedByChannel[channel.id] ?? 0;
    if (event.createdAt > current) {
      _latestObservedByChannel[channel.id] = event.createdAt;
    }
    return true;
  }
}

/// Directory edits for readers outside the notifier.
extension ChannelDirectoryForgetting on ChannelsNotifier {
  /// Drops an unjoined channel the relay no longer serves from the
  /// directory, so readers stop offering it before the next full read.
  void forgetDirectoryChannel(String channelId) =>
      _forgetDirectoryChannel(channelId);
}

/// Activity projection of the channel unread registry.
extension ObservedThreadActivity on ChannelsNotifier {
  /// The same eligible replies that supply channel attention dots. Consumers
  /// watch [channelsProvider] for changes; this does not fetch extra history.
  Iterable<NostrEvent> observedThreadReplies(Iterable<Channel> channels) sync* {
    for (final channel in channels) {
      if (!channel.isMember || channel.isArchived || channel.isDm) continue;
      for (final observed
          in _observedUnreadEventsByChannel[channel.id]?.values ??
              const <ObservedUnreadEvent>[]) {
        if (observed.threadReply case final reply?) yield reply;
      }
    }
  }
}

/// How long a saved directory serves readers before its full read again,
/// which is the only way channels that turned private, were deleted or were
/// unlisted leave it: the relay never tells a non-member about those.
const savedDirectoryFullReadAge = Duration(hours: 24);

/// How often directory readers may ask the relay for changes.
const savedDirectoryCheckInterval = Duration(minutes: 1);

/// How far before the newest saved change a catch-up starts, so changes the
/// relay stamped in the same second as it are not missed.
const savedDirectoryCatchUpOverlap = Duration(minutes: 1);

/// The latest revision of each channel in [metas], kept only while it is
/// discoverable: open, listed and not a DM. The relay's directory read also
/// returns the caller's own private, unlisted and DM channels, which must
/// never be held or saved as the open-channel directory. A channel whose
/// latest revision is no longer discoverable drops out.
List<NostrEvent> _discoverableChannelEvents(List<NostrEvent> metas) => [
  for (final event in _latestChannelEvents(metas, kind: 39000))
    if (ChannelData.fromEvent(event) case final data
        when data.visibility != 'private' &&
            data.channelType != 'dm' &&
            !data.isUnlisted)
      event,
];

/// Whether [elapsed] is less than [interval] and not negative. A negative
/// elapsed time means the clock was set back since, so the interval counts as
/// over rather than lasting until the clock catches up.
bool _isWithin(Duration elapsed, Duration interval) =>
    !elapsed.isNegative && elapsed < interval;

/// The clock the saved directory's full-read and check intervals use.
final channelDirectoryClockProvider = Provider<DateTime Function()>(
  (ref) => DateTime.now,
);

/// Which scope's directory the notifier holds, and when it was last read.
class _SavedDirectory {
  /// The relay-and-identity scope [ChannelsNotifier._directoryMetas] holds,
  /// or null when it holds none.
  String? scope;

  /// When the relay last served the whole directory, in Unix seconds.
  int? fullAt;

  /// When the last read that changed the directory started, by the device
  /// clock, in Unix seconds. It caps the catch-up cursor: the relay stamps a
  /// channel's metadata at least one second past its previous revision, so a
  /// burst of joins or edits can date it minutes ahead of when it was served.
  int? readAt;

  /// When a reader last asked the relay for changes.
  DateTime? checkedAt;

  /// Whether a reader opened while the session was not connected, so the
  /// next connect checks the relay for it.
  bool checkOnConnect = false;

  /// The relay read a reader started, until it settles. Readers that open
  /// meanwhile share it, so a read slower than the check interval is not
  /// displaced by another.
  Future<void>? checking;

  void reset() {
    scope = null;
    fullAt = null;
    readAt = null;
    checkedAt = null;
    checkOnConnect = false;
    checking = null;
  }
}

/// The saved open-channel directory: restoring it, keeping it current, and
/// the list edits directory loads share with full refreshes.
///
/// Lives in this part file because `channels_provider.dart` sits against the
/// repository-wide 1200-line file ceiling enforced by `just file-size-check`.
extension _ChannelsNotifierDirectory on ChannelsNotifier {
  /// Installs the discoverable channels in [served] as this scope's
  /// directory and saves them. A read started at [readStartedAt] dates
  /// them; a [full] read replaces the directory and restarts its full-read
  /// age and check interval.
  void _installDirectory(
    List<NostrEvent> served,
    String scope, {
    DateTime? readStartedAt,
    bool full = false,
  }) {
    final now = _lifecycleRef.read(channelDirectoryClockProvider)();
    final metas = _directoryMetas = _discoverableChannelEvents(served);
    _savedDirectory.scope = scope;
    if (readStartedAt != null) {
      _savedDirectory.readAt = readStartedAt.millisecondsSinceEpoch ~/ 1000;
    }
    if (full) {
      _savedDirectory
        ..fullAt = now.millisecondsSinceEpoch ~/ 1000
        ..checkedAt = now;
    }
    final _SavedDirectory(:fullAt, :readAt) = _savedDirectory;
    if (fullAt == null || readAt == null) return;
    // The scope fence was checked synchronously before this call, so the
    // storage read here belongs to the same community and account.
    _lifecycleRef
        .read(openChannelDirectoryStorageProvider)
        ?.write(
          SavedOpenChannelDirectory(
            metas: metas,
            fullAt: fullAt,
            readAt: readAt,
          ),
        )
        .catchError((Object error) {
          // The copy is a cache: the next change or full read saves it again.
          debugPrint('[ChannelsNotifier] saving the directory failed: $error');
          return false;
        });
  }

  /// Installs a read of the whole directory as this scope's directory.
  void _installFullRead(_DirectoryRead read, String scope) => _installDirectory(
    read.metas,
    scope,
    readStartedAt: read.startedAt,
    full: true,
  );

  /// Restores this scope's saved directory into the list, unless the
  /// notifier already holds it. Returns whether a directory is now held.
  Future<bool> _restoreSavedDirectory(String scope) async {
    if (_savedDirectory.scope == scope) return true;
    final storage = _lifecycleRef.read(openChannelDirectoryStorageProvider);
    final fence = _refreshCoordinator.beginDirectoryOnly();
    final saved = await storage?.read();
    try {
      fence.ensureCurrent();
    } on _StaleChannelRefresh {
      return false;
    }
    // A load that finished meanwhile is newer than the file.
    if (_savedDirectory.scope == scope) return true;
    if (saved == null) return false;
    // Filtered again, so a copy saved under older rules holds nothing the
    // current rules exclude.
    _directoryMetas = _discoverableChannelEvents(saved.metas);
    _savedDirectory
      ..scope = scope
      ..fullAt = saved.fullAt
      ..readAt = saved.readAt;
    // A load that started meanwhile owns the status until it settles.
    final status = _refreshCoordinator.loadStatus();
    if (!status.isLoading(scope)) status.markLoaded(scope);
    _mergeDirectoryIntoList();
    return true;
  }

  /// Rebuilds the list's unjoined channels from [_directoryMetas] and saves
  /// the member snapshot, when the list has loaded.
  void _mergeDirectoryIntoList() {
    // While a list load runs or after it failed, the value can still be the
    // previous community's or account's list. The load merges the held
    // directory itself when it lands.
    if (_listState.isLoading || _listState.hasError) return;
    final previous = _listState.value;
    if (previous == null) return;
    final channels = [...previous];
    _replaceDirectoryChannels(channels);
    _sortChannels(channels);
    _invalidateChangedDetails(channels, previous);
    _listState = AsyncData(channels);
    _saveSnapshot?.call(channels, _hiddenDmIds);
  }

  /// Checks a held directory for changes in the background now, or on the
  /// next connect when the session is not connected.
  void _checkSavedDirectoryWhenConnected() {
    final connected =
        _lifecycleRef.read(relaySessionProvider).status ==
        SessionStatus.connected;
    _savedDirectory.checkOnConnect = !connected;
    if (!connected) return;
    unawaited(
      _checkSavedDirectory().catchError((Object error) {
        debugPrint('[ChannelsNotifier] directory check failed: $error');
      }),
    );
  }

  /// Runs the check a reader asked for while the session was not connected.
  void _checkSavedDirectoryOnConnect() {
    if (_savedDirectory.checkOnConnect) _checkSavedDirectoryWhenConnected();
  }

  /// Reads the relay for a held directory, at most once per
  /// [savedDirectoryCheckInterval]: the whole directory once the last full
  /// read is older than [savedDirectoryFullReadAge], otherwise only what
  /// changed since the newest saved change. Both are quiet: readers keep
  /// showing the held directory meanwhile.
  /// A reader opening while a read is in flight shares it.
  Future<void> _checkSavedDirectory() {
    final inFlight = _savedDirectory.checking;
    if (inFlight != null) return inFlight;
    final now = _lifecycleRef.read(channelDirectoryClockProvider)();
    final checkedAt = _savedDirectory.checkedAt;
    // A check dated in the future, by a clock since set back, is due.
    if (checkedAt != null &&
        _isWithin(now.difference(checkedAt), savedDirectoryCheckInterval)) {
      return Future.value();
    }
    _savedDirectory.checkedAt = now;
    late final Future<void> check;
    check = _readSavedDirectoryChanges(now).whenComplete(() {
      if (identical(_savedDirectory.checking, check)) {
        _savedDirectory.checking = null;
      }
    });
    return _savedDirectory.checking = check;
  }

  Future<void> _readSavedDirectoryChanges(DateTime now) async {
    final fullAt = _savedDirectory.fullAt;
    // A full read dated in the future, by a clock since set back, is due.
    final fullReadDue =
        fullAt == null ||
        !_isWithin(
          Duration(seconds: now.millisecondsSinceEpoch ~/ 1000 - fullAt),
          savedDirectoryFullReadAge,
        );
    int? since;
    if (!fullReadDue) {
      final newest = _directoryMetas.fold<int>(
        0,
        (newest, event) => max(newest, event.createdAt),
      );
      // A channel created since the last read may carry an earlier stamp
      // than the newest held one, so the catch-up also starts before that
      // read. Its start is device time and the stamps are relay time, so it
      // reaches back by the clock skew the relay tolerates.
      final readAt = _savedDirectory.readAt;
      since = max(
        0,
        min(
          newest - savedDirectoryCatchUpOverlap.inSeconds,
          readAt == null ? newest : readAt - relayCrossDeviceSkewSeconds,
        ),
      );
    }
    final fence = _refreshCoordinator.beginDirectoryOnly();
    try {
      final read = await _refreshCoordinator.loadDirectory(
        _lifecycleRef.read(relaySessionProvider.notifier),
        fence,
        since: since,
        quiet: true,
      );
      if (read == null) return;
      if (since == null) {
        _installFullRead(read, fence.scope);
      } else {
        // The overlap always brings back the newest saved event; a catch-up
        // that leaves the held events as they are changed nothing, so nothing
        // is saved or rebuilt.
        final held = {for (final event in _directoryMetas) event.id};
        final merged = _discoverableChannelEvents([
          ..._directoryMetas,
          ...read.metas,
        ]);
        if (merged.length == held.length &&
            merged.every((event) => held.contains(event.id))) {
          _savedDirectory.readAt =
              read.startedAt.millisecondsSinceEpoch ~/ 1000;
          return;
        }
        _installDirectory(merged, fence.scope, readStartedAt: read.startedAt);
      }
    } on _StaleChannelRefresh {
      return;
    }
    _mergeDirectoryIntoList();
  }

  /// Drops [channelId] from the held directory and from the list's unjoined
  /// channels, as when opening it finds the relay no longer serves it.
  void _forgetDirectoryChannel(String channelId) {
    final scope = _savedDirectory.scope;
    if (scope == null ||
        !_directoryMetas.any((event) => event.getTagValue('d') == channelId)) {
      return;
    }
    _refreshCoordinator.forget(channelId);
    _installDirectory([
      for (final event in _directoryMetas)
        if (event.getTagValue('d') != channelId) event,
    ], scope);
    _mergeDirectoryIntoList();
  }

  /// Downloads only the open-channel directory and merges it into the list
  /// [current] returns once it lands, for a list that already has its
  /// memberships. Joined channels keep their entries; live updates keep them
  /// current. Returns null when nothing should be written: the load failed
  /// (its status says so), a newer directory load replaced it, the scope
  /// changed, or there is no list to merge into.
  Future<List<Channel>?> _loadDirectoryOnly(
    List<Channel>? Function() current,
  ) async {
    final fence = _refreshCoordinator.beginDirectoryOnly();
    try {
      final read = await _refreshCoordinator.loadDirectory(
        _lifecycleRef.read(relaySessionProvider.notifier),
        fence,
      );
      if (read == null) return null;
      _installFullRead(read, fence.scope);
    } on _StaleChannelRefresh {
      return null;
    }
    final previous = current();
    if (previous == null) return null;
    final channels = [...previous];
    _replaceDirectoryChannels(channels);
    _sortChannels(channels);
    _invalidateChangedDetails(channels, previous);
    return channels;
  }

  /// Rebuilds every channel in [channels] the user is not a member of from
  /// [_directoryMetas], with a full refresh's visibility rules: the latest
  /// revision of each open, non-DM channel. Joined channels are untouched.
  void _replaceDirectoryChannels(List<Channel> channels) {
    final previous = {
      for (final channel in channels)
        if (!channel.isMember) channel.id: channel,
    };
    channels.removeWhere((channel) => !channel.isMember);
    final memberIds = {for (final channel in channels) channel.id};
    for (final event in _latestChannelEvents(_directoryMetas, kind: 39000)) {
      final data = ChannelData.fromEvent(event);
      if (memberIds.contains(data.id) ||
          data.visibility == 'private' ||
          data.channelType == 'dm') {
        continue;
      }
      final channel = _channelFromMeta(event, data, isMember: false);
      final lastMessageAt = previous[data.id]?.lastMessageAt;
      channels.add(
        lastMessageAt == null
            ? channel
            : channel.copyWith(lastMessageAt: lastMessageAt),
      );
    }
  }

  /// Invalidates the details of channels whose identity or archive state
  /// changed from [previous], so opening one does not show stale Solo or
  /// archive state. Returns [previous] by id.
  Map<String, Channel> _invalidateChangedDetails(
    List<Channel> channels,
    List<Channel>? previous,
  ) {
    final prevById = <String, Channel>{
      for (final c in previous ?? const <Channel>[]) c.id: c,
    };
    for (final channel in channels) {
      final prev = prevById[channel.id];
      if (prev == null ||
          prev.isArchived != channel.isArchived ||
          prev.channelType != channel.channelType ||
          prev.visibility != channel.visibility ||
          prev.createdBy != channel.createdBy ||
          !listEquals(prev.labels, channel.labels)) {
        _lifecycleRef.invalidate(channelDetailsProvider(channel.id));
      }
    }
    return prevById;
  }
}

/// Orders the list by channel type, then case-insensitively by name to match
/// desktop's `localeCompare` ordering.
void _sortChannels(List<Channel> channels) => channels.sort((left, right) {
  final typeOrder =
      (_channelTypeOrder[left.channelType] ?? 99) -
      (_channelTypeOrder[right.channelType] ?? 99);
  if (typeOrder != 0) return typeOrder;
  return left.name.toLowerCase().compareTo(right.name.toLowerCase());
});
