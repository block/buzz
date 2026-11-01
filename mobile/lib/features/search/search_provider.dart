import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/relay/relay.dart';
import '../channels/channel.dart';
import '../channels/channel_management_provider.dart';
import '../channels/channels_provider.dart';
import 'name_match.dart';

@immutable
class SearchHit {
  final String eventId;
  final String content;
  final int kind;
  final String pubkey;
  final String? channelId;
  final String? channelName;
  final int createdAt;
  final double score;
  final List<List<String>> tags;

  const SearchHit({
    required this.eventId,
    required this.content,
    required this.kind,
    required this.pubkey,
    this.channelId,
    this.channelName,
    required this.createdAt,
    required this.score,
    this.tags = const [],
  });

  factory SearchHit.fromJson(Map<String, dynamic> json) => SearchHit(
    eventId: json['event_id'] as String,
    content: json['content'] as String? ?? '',
    kind: json['kind'] as int? ?? 9,
    pubkey: json['pubkey'] as String,
    channelId: json['channel_id'] as String?,
    channelName: json['channel_name'] as String?,
    createdAt: json['created_at'] as int? ?? 0,
    score: (json['score'] as num?)?.toDouble() ?? 0.0,
    tags:
        (json['tags'] as List<dynamic>?)
            ?.map((t) => (t as List<dynamic>).map((e) => e as String).toList())
            .toList() ??
        const [],
  );
}

@immutable
class SearchState {
  final String query;
  final List<SearchHit> messageResults;
  final List<DirectoryUser> userResults;
  final List<Channel> channelResults;
  final bool isLoading;
  final String? error;

  /// The open-channel directory failed to load, so channel results may be
  /// missing open channels the user has not joined.
  final bool channelSearchFailed;

  const SearchState({
    this.query = '',
    this.messageResults = const [],
    this.userResults = const [],
    this.channelResults = const [],
    this.isLoading = false,
    this.error,
    this.channelSearchFailed = false,
  });

  const SearchState.initial()
    : query = '',
      messageResults = const [],
      userResults = const [],
      channelResults = const [],
      isLoading = false,
      error = null,
      channelSearchFailed = false;

  SearchState copyWith({
    String? query,
    List<SearchHit>? messageResults,
    List<DirectoryUser>? userResults,
    List<Channel>? channelResults,
    bool? isLoading,
    String? error,
    bool? channelSearchFailed,
  }) => SearchState(
    query: query ?? this.query,
    messageResults: messageResults ?? this.messageResults,
    userResults: userResults ?? this.userResults,
    channelResults: channelResults ?? this.channelResults,
    isLoading: isLoading ?? this.isLoading,
    error: error ?? this.error,
    channelSearchFailed: channelSearchFailed ?? this.channelSearchFailed,
  );
}

/// The channel name text a search [query] matches: lowercased, without a
/// leading "#".
String _channelNeedle(String query) =>
    query.trim().toLowerCase().replaceFirst(RegExp(r'^#'), '');

/// Whether [query] searches open channels. A bare "#" does not, as on desktop.
bool _searchesOpenChannels(String query) => _channelNeedle(query).isNotEmpty;

/// Most channel rows one query shows, ranked best first.
const searchChannelResultLimit = 8;

/// How long a loaded open-channel directory serves searches before the next
/// search reads it again, as desktop's `PUBLIC_CHANNEL_PAGE_TTL` does.
const searchDirectoryMaxAge = Duration(seconds: 30);

/// Ranks [channels] for a search [query] with the same rules as desktop
/// (buzz-app `SearchResults.tsx` and `store.ts` `searchPublic`): name match
/// rank first, an archived channel half a step behind a live one, then joined
/// channels in list order before open ones by name. A bare "#" lists joined
/// channels only, since desktop does not search open channels for it.
List<Channel> rankSearchChannels(Iterable<Channel> channels, String query) {
  final needle = _channelNeedle(query);
  final ranked = <({Channel channel, double rank, int order})>[];
  for (final channel in channels) {
    if (channel.isDm) continue;
    // An archived channel stays findable only by its members.
    final visible = channel.isArchived || needle.isEmpty
        ? channel.isMember
        : channel.visibility == 'open' || channel.isMember;
    if (!visible) continue;
    final rank = matchRank(channel.name, needle);
    if (rank == null) continue;
    ranked.add((
      channel: channel,
      rank: rank + (channel.isArchived ? 0.5 : 0),
      order: ranked.length,
    ));
  }
  ranked.sort((a, b) {
    final byRank = a.rank.compareTo(b.rank);
    if (byRank != 0) return byRank;
    // Joined channels win ties and keep the list's order.
    if (a.channel.isMember != b.channel.isMember) {
      return a.channel.isMember ? -1 : 1;
    }
    if (a.channel.isMember) return a.order.compareTo(b.order);
    final byName = a.channel.name.compareTo(b.channel.name);
    return byName != 0 ? byName : a.channel.id.compareTo(b.channel.id);
  });
  return [
    for (final entry in ranked.take(searchChannelResultLimit)) entry.channel,
  ];
}

class SearchNotifier extends Notifier<SearchState> {
  Timer? _debounce;

  // The relay lookups still running for the current query. The spinner shows
  // only while one of them, or a directory load, is pending, so the last
  // lookup to finish settles it regardless of which results came back empty.
  bool _messagesPending = false;
  bool _usersPending = false;

  // The directory load this search started, until its future completes. The
  // shared directory status is not enough: production marks it loading only
  // after memberships arrive, and loaded before the new list is installed.
  // A rebuild drops it, so a load from an earlier community cannot settle a
  // newer one.
  Object? _directoryLoad;

  // This search has loaded, or tried to load, the directory. Until then an
  // unloaded directory is not a failure to report.
  bool _directoryAttempted = false;

  // Bumped by every new query, clear and rebuild. A lookup writes its result
  // only while its generation is current, so a late reply to an earlier query,
  // even one with the same text, cannot settle the current spinner.
  int _generation = 0;

  @override
  SearchState build() {
    // Watch relayConfig so search state resets on a community or identity
    // switch: myPubkeyProvider derives from its nsec.
    ref.watch(relayConfigProvider);
    _generation++;
    _messagesPending = false;
    _usersPending = false;
    _directoryLoad = null;
    _directoryAttempted = false;
    // Channel rows come from the loaded channel list, which gains open
    // channels when the directory arrives. Re-rank whenever either changes;
    // the directory status for this community also drives the spinner.
    ref.listen(channelsProvider, (_, _) => _refreshChannels());
    ref.listen(
      channelDirectoryLoadStatusProvider,
      (_, _) => _refreshChannels(),
    );
    ref.onDispose(() {
      _debounce?.cancel();
    });
    return const SearchState.initial();
  }

  void search(String query) {
    _debounce?.cancel();

    final trimmed = query.trim();
    if (trimmed.isEmpty) {
      clear();
      return;
    }

    final generation = ++_generation;
    // Pending from the keystroke, so a channel-list change during the
    // debounce cannot show "No results" before the lookups start.
    _messagesPending = true;
    _usersPending = true;
    state = SearchState(query: trimmed, isLoading: true);

    _debounce = Timer(const Duration(milliseconds: 300), () {
      _executeSearch(trimmed, generation);
    });
  }

  void _executeSearch(String query, int generation) {
    if (generation != _generation) return;

    // Like desktop, a bare "#" lists joined channels without the directory.
    if (_searchesOpenChannels(query)) _loadDirectory();
    _refreshChannels();
    _searchMessages(query, generation);
    _searchUsers(query, generation);
  }

  bool get _isLoading =>
      _messagesPending ||
      _usersPending ||
      (_searchesOpenChannels(state.query) &&
          (_directoryLoad != null ||
              _directoryStatus == ChannelDirectoryLoadStatus.loading));

  /// This community's directory status, or null when the status describes
  /// another community or no load has started.
  ChannelDirectoryLoadStatus? get _directoryStatus {
    final directory = ref.read(channelDirectoryLoadStatusProvider);
    final scope = channelDirectoryScope(
      ref.read(relayConfigProvider).baseUrl,
      ref.read(myPubkeyProvider),
    );
    return directory.scope == scope ? directory.status : null;
  }

  /// Whether this community's directory is not loaded and no load for it is
  /// running: it failed, or a newer channel refresh retired the load.
  bool get _directoryMissing => switch (_directoryStatus) {
    ChannelDirectoryLoadStatus.loaded ||
    ChannelDirectoryLoadStatus.loading => false,
    _ => true,
  };

  /// Loads the directory again after it failed, like desktop's Retry channels.
  void retryChannels() {
    _loadDirectory();
    _refreshChannels();
  }

  /// Loads the open-channel directory so search also finds open channels the
  /// user has not joined, as desktop search does.
  void _loadDirectory() {
    if (_directoryLoad != null) return;
    final load = _directoryLoad = Object();
    _directoryAttempted = true;
    final channels = ref.read(channelsProvider.notifier);
    Future<void> ensure() =>
        channels.ensureDirectoryLoaded(maxAge: searchDirectoryMaxAge);
    unawaited(
      ensure()
          .then((_) {
            // A membership refresh that starts during the load retires it.
            // Nothing failed, so the load ends with no status for this
            // community, or idle. Load it again once; if that is retired
            // too, search offers Retry instead of looping. A relay error
            // ends in error and is not retried automatically.
            if (!ref.mounted ||
                _directoryLoad != load ||
                !_directoryMissing ||
                _directoryStatus == ChannelDirectoryLoadStatus.error) {
              return null;
            }
            return ensure();
          })
          .catchError((Object error) {
            debugPrint(
              '[SearchNotifier] channel directory load failed: $error',
            );
          })
          .whenComplete(() {
            if (!ref.mounted || _directoryLoad != load) return;
            _directoryLoad = null;
            _refreshChannels();
          }),
    );
  }

  Future<void> _searchMessages(String query, int generation) async {
    try {
      final session = ref.read(relaySessionProvider.notifier);
      final events = await session.fetchHistory(
        NostrFilter(
          kinds: const [9, 40002, 45001, 45003],
          search: query,
          limit: 20,
        ),
      );

      // Channel-name lookup from the cached channels list — not all hits
      // will have a known channel (e.g. ones we're not a member of).
      final channelsById = {
        for (final c
            in ref.read(channelsProvider).asData?.value ?? const <Channel>[])
          c.id: c.name,
      };

      final hits = events
          .map(
            (e) => SearchHit(
              eventId: e.id,
              content: e.content,
              kind: e.kind,
              pubkey: e.pubkey,
              channelId: e.channelId,
              channelName: e.channelId != null
                  ? channelsById[e.channelId!]
                  : null,
              createdAt: e.createdAt,
              score: 0.0,
              tags: e.tags,
            ),
          )
          .toList();

      if (!ref.mounted || generation != _generation) return;
      _messagesPending = false;
      state = state.copyWith(messageResults: hits, isLoading: _isLoading);
    } catch (e) {
      if (!ref.mounted || generation != _generation) return;
      _messagesPending = false;
      state = state.copyWith(isLoading: _isLoading, error: e.toString());
    }
  }

  Future<void> _searchUsers(String query, int generation) async {
    List<DirectoryUser>? users;
    try {
      users = await ref
          .read(channelActionsProvider)
          .searchUsers(query, limit: 8);
    } catch (_) {
      // User search failure is non-critical — keep existing results.
    }
    if (!ref.mounted || generation != _generation) return;
    _usersPending = false;
    state = state.copyWith(userResults: users, isLoading: _isLoading);
  }

  /// Publishes the channel rows and spinner for the current query.
  void _refreshChannels() {
    if (state.query.isEmpty) return;
    state = state.copyWith(
      // A reload keeps the previous community's list in AsyncLoading; only
      // installed data belongs to this search's community.
      channelResults: rankSearchChannels(
        ref.read(channelsProvider).asData?.value ?? const <Channel>[],
        state.query,
      ),
      // An earlier failure stays hidden while our next load runs, and the
      // notice clears when another caller loads the directory.
      channelSearchFailed:
          _searchesOpenChannels(state.query) &&
          _directoryAttempted &&
          _directoryLoad == null &&
          _directoryMissing,
      isLoading: _isLoading,
    );
  }

  void clear() {
    _debounce?.cancel();
    _generation++;
    _messagesPending = false;
    _usersPending = false;
    state = const SearchState.initial();
  }
}

final searchProvider = NotifierProvider<SearchNotifier, SearchState>(
  SearchNotifier.new,
);
