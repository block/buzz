import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/relay/relay.dart';
import '../channels/channel.dart';
import '../channels/channel_management_provider.dart';
import '../channels/channels_provider.dart';

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

class SearchNotifier extends Notifier<SearchState> {
  Timer? _debounce;
  int _generation = 0;
  int _scopeGeneration = 0;
  bool _lookupsPending = false;
  bool _channelLookupFailed = false;
  Future<void>? _directoryLoad;

  @override
  SearchState build() {
    // Watch relayConfig so search state resets on community switch.
    ref.watch(relayConfigProvider);
    ref.watch(myPubkeyProvider);
    _directoryLoad = null;
    _lookupsPending = false;
    _channelLookupFailed = false;
    ref.listen(channelsProvider, (_, _) => _updateChannels());
    ref.listen(channelDirectoryLoadStatusProvider, (_, _) => _updateChannels());
    ref.onDispose(() {
      _debounce?.cancel();
      _generation++;
      _scopeGeneration++;
    });
    return const SearchState.initial();
  }

  void search(String query) {
    _debounce?.cancel();

    final generation = ++_generation;
    final trimmed = query.trim();
    if (trimmed.isEmpty) {
      state = const SearchState.initial();
      return;
    }

    _lookupsPending = true;
    _channelLookupFailed = false;
    state = SearchState(query: trimmed, isLoading: true);

    _debounce = Timer(const Duration(milliseconds: 300), () {
      _executeSearch(trimmed, generation);
    });
  }

  bool _isCurrent(int generation) => ref.mounted && generation == _generation;

  Future<void> _executeSearch(String query, int generation) async {
    if (!_isCurrent(generation)) return;
    _updateChannels();
    await Future.wait([
      _searchMessages(query, generation),
      _searchUsers(query, generation),
      _searchChannels(generation),
    ]);
    if (!_isCurrent(generation)) return;
    _lookupsPending = false;
    _updateChannels();
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

      if (!_isCurrent(generation)) return;

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

      if (!_isCurrent(generation)) return;
      state = state.copyWith(messageResults: hits);
    } catch (e) {
      if (!_isCurrent(generation)) return;
      state = state.copyWith(error: 'Couldn’t load messages.');
    }
  }

  Future<void> _searchUsers(String query, int generation) async {
    try {
      final users = await ref
          .read(channelActionsProvider)
          .searchUsers(query, limit: 8);

      if (!_isCurrent(generation)) return;
      state = state.copyWith(userResults: users);
    } catch (_) {
      // User search failure is non-critical — keep existing results.
    }
  }

  Future<void> _loadDirectory() async {
    final scopeGeneration = _scopeGeneration;
    if (ref.read(relaySessionProvider).status != SessionStatus.connected) {
      throw StateError('Channel discovery requires a connection');
    }
    // Wait for membership startup before discovery takes refresh ownership.
    // A failed startup can be recovered by the directory refresh below.
    try {
      await ref.read(channelsProvider.future);
    } catch (_) {
      // ensureDirectoryLoaded retries membership loading too.
    }
    if (!ref.mounted || scopeGeneration != _scopeGeneration) return;
    final channels = ref.read(channelsProvider.notifier);
    await channels.ensureDirectoryLoaded();
    if (!ref.mounted || scopeGeneration != _scopeGeneration) return;
    if (ref.read(channelDirectoryLoadStatusProvider).status ==
        ChannelDirectoryLoadStatus.idle) {
      // A membership refresh displaced discovery. One retry per shared
      // request, never a loop and never an automatic retry of relay errors.
      await channels.ensureDirectoryLoaded();
    }
  }

  Future<void> _searchChannels(int generation) async {
    try {
      final request = _directoryLoad ??= _loadDirectory();
      try {
        await request;
      } finally {
        if (identical(_directoryLoad, request)) _directoryLoad = null;
      }
    } catch (_) {
      if (!_isCurrent(generation)) return;
      _channelLookupFailed = true;
    }
  }

  void _updateChannels() {
    if (!ref.mounted || state.query.isEmpty) return;
    // A reload can retain the previous community’s value in AsyncLoading.
    // Only installed data belongs to the current search scope.
    final channels = ref.read(channelsProvider).asData?.value ?? [];
    final lowerQuery = state.query.toLowerCase();
    final matches = channels.where((c) {
      if (c.isDm) return false;
      return c.name.toLowerCase().contains(lowerQuery) ||
          c.description.toLowerCase().contains(lowerQuery);
    }).toList();
    final directory = ref.read(channelDirectoryLoadStatusProvider);
    final scope = channelDirectoryScope(
      ref.read(relayConfigProvider).baseUrl,
      ref.read(myPubkeyProvider),
    );
    final status = directory.scope == scope ? directory.status : null;
    if (status == ChannelDirectoryLoadStatus.loaded) {
      _channelLookupFailed = false;
    }
    state = state.copyWith(
      channelResults: matches,
      isLoading:
          _lookupsPending || status == ChannelDirectoryLoadStatus.loading,
      channelSearchFailed:
          !_lookupsPending &&
          (_channelLookupFailed ||
              status == ChannelDirectoryLoadStatus.error ||
              status == ChannelDirectoryLoadStatus.idle),
    );
  }

  void clear() {
    _generation++;
    _debounce?.cancel();
    state = const SearchState.initial();
  }
}

final searchProvider = NotifierProvider<SearchNotifier, SearchState>(
  SearchNotifier.new,
);
