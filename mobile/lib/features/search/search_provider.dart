import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/relay/relay.dart';
import '../channels/channel.dart';
import '../channels/channel_management_provider.dart';
import '../channels/channels_provider.dart';
import 'channel_search_score.dart';

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

  const SearchState({
    this.query = '',
    this.messageResults = const [],
    this.userResults = const [],
    this.channelResults = const [],
    this.isLoading = false,
    this.error,
  });

  const SearchState.initial()
    : query = '',
      messageResults = const [],
      userResults = const [],
      channelResults = const [],
      isLoading = false,
      error = null;

  SearchState copyWith({
    String? query,
    List<SearchHit>? messageResults,
    List<DirectoryUser>? userResults,
    List<Channel>? channelResults,
    bool? isLoading,
    String? error,
  }) => SearchState(
    query: query ?? this.query,
    messageResults: messageResults ?? this.messageResults,
    userResults: userResults ?? this.userResults,
    channelResults: channelResults ?? this.channelResults,
    isLoading: isLoading ?? this.isLoading,
    error: error ?? this.error,
  );
}

/// Most channel rows one query shows, ranked best first.
const searchChannelResultLimit = 20;

class SearchNotifier extends Notifier<SearchState> {
  Timer? _debounce;

  // The relay lookups still running for the current query. The spinner shows
  // only while one of them is pending, so the last lookup to finish settles it
  // regardless of which results came back empty.
  bool _messagesPending = false;
  bool _usersPending = false;
  bool _directoryPending = false;

  @override
  SearchState build() {
    // Watch relayConfig so search state resets on community switch.
    ref.watch(relayConfigProvider);
    _messagesPending = false;
    _usersPending = false;
    _directoryPending = false;
    // Channel rows come from the loaded channel list, which gains open
    // channels when the directory arrives. Re-rank whenever either changes.
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

    _messagesPending = false;
    _usersPending = false;
    state = SearchState(query: trimmed, isLoading: true);

    _debounce = Timer(const Duration(milliseconds: 300), () {
      _executeSearch(trimmed);
    });
  }

  Future<void> _executeSearch(String query) async {
    // If the query changed while debouncing, bail out.
    if (state.query != query) return;

    _messagesPending = true;
    _usersPending = true;
    _loadDirectory();
    _refreshChannels();
    _searchMessages(query);
    _searchUsers(query);
  }

  bool get _isLoading =>
      _messagesPending ||
      _usersPending ||
      _directoryPending ||
      ref
          .read(channelDirectoryLoadStatusProvider.notifier)
          .isLoading(
            channelDirectoryScope(
              ref.read(relayConfigProvider).baseUrl,
              ref.read(myPubkeyProvider),
            ),
          );

  /// Loads the open-channel directory so search also finds open channels the
  /// user has not joined, as desktop search does.
  void _loadDirectory() {
    if (_directoryPending) return;
    _directoryPending = true;
    unawaited(
      ref
          .read(channelsProvider.notifier)
          .ensureDirectoryLoaded()
          .catchError((Object error) {
            debugPrint(
              '[SearchNotifier] channel directory load failed: $error',
            );
          })
          .whenComplete(() {
            if (!ref.mounted) return;
            _directoryPending = false;
            _refreshChannels();
          }),
    );
  }

  Future<void> _searchMessages(String query) async {
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
        for (final c in ref.read(channelsProvider).value ?? const <Channel>[])
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

      if (!ref.mounted || state.query != query) return;
      _messagesPending = false;
      state = state.copyWith(messageResults: hits, isLoading: _isLoading);
    } catch (e) {
      if (!ref.mounted || state.query != query) return;
      _messagesPending = false;
      state = state.copyWith(isLoading: _isLoading, error: e.toString());
    }
  }

  Future<void> _searchUsers(String query) async {
    List<DirectoryUser>? users;
    try {
      users = await ref
          .read(channelActionsProvider)
          .searchUsers(query, limit: 8);
    } catch (_) {
      // User search failure is non-critical — keep existing results.
    }
    if (!ref.mounted || state.query != query) return;
    _usersPending = false;
    state = state.copyWith(userResults: users, isLoading: _isLoading);
  }

  /// Ranks the loaded channels against the current query with the same rules
  /// as desktop search.
  void _refreshChannels() {
    final query = state.query;
    if (query.isEmpty) return;
    final lowerQuery = query.toLowerCase();
    final channels = ref.read(channelsProvider).value ?? const <Channel>[];
    final ranked = <({Channel channel, int score})>[];
    for (final channel in channels) {
      if (channel.isDm) continue;
      // An archived channel stays findable only by its members.
      final visible = channel.isArchived
          ? channel.isMember
          : channel.visibility == 'open' || channel.isMember;
      if (!visible) continue;
      final score = scoreChannelMatch(
        name: channel.name,
        description: channel.description,
        lowerQuery: lowerQuery,
      );
      if (score == null) continue;
      ranked.add((channel: channel, score: score));
    }
    ranked.sort((a, b) {
      final byScore = a.score.compareTo(b.score);
      if (byScore != 0) return byScore;
      return a.channel.name.toLowerCase().compareTo(
        b.channel.name.toLowerCase(),
      );
    });
    state = state.copyWith(
      channelResults: [
        for (final entry in ranked.take(searchChannelResultLimit))
          entry.channel,
      ],
      isLoading: _isLoading,
    );
  }

  void clear() {
    _debounce?.cancel();
    _messagesPending = false;
    _usersPending = false;
    state = const SearchState.initial();
  }
}

final searchProvider = NotifierProvider<SearchNotifier, SearchState>(
  SearchNotifier.new,
);
