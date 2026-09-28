import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../../shared/crypto/nip_oa.dart';
import '../../../shared/identity_names/identity_names.dart';
import '../../../shared/identity_names/identity_names_provider.dart';
import '../../../shared/mentions/agent_identity_provider.dart';
import '../../../shared/relay/relay.dart';
import '../../../shared/profile/user_cache_provider.dart';
import '../../../shared/profile/user_profile.dart';
import '../channel.dart';
import '../channel_management_provider.dart';
import '../channels_provider.dart';
import 'mention_candidates.dart';
import 'mention_ranking.dart';
import '../../../shared/mentions/mention_rules.dart';
import '../../../shared/mentions/mention_tags.dart';
import '../../profile/presence_cache_provider.dart';

/// Typing pause before a mention query hits the relay search endpoint.
const mentionSearchDebounce = Duration(milliseconds: 200);

/// Global user search for mention autocomplete — kind:0 prefix search via
/// the relay HTTP bridge. Mirrors desktop's `useInfiniteUserSearchQuery`
/// feeding `useMentions` (source 4: people and agents outside the channel).
///
/// Debounced: the provider waits [mentionSearchDebounce] before querying;
/// keystrokes dispose the stale family member so its request never fires.
final mentionUserSearchProvider = FutureProvider.autoDispose
    .family<List<UserProfile>, String>((ref, query) async {
      final trimmed = query.trim();
      if (trimmed.isEmpty) return const [];

      var disposed = false;
      ref.onDispose(() => disposed = true);
      await Future<void>.delayed(mentionSearchDebounce);
      if (disposed) return const [];

      final session = ref.read(relaySessionProvider.notifier);
      final events = await session.queryRelay([
        NostrFilters.searchUsers(trimmed),
      ]);

      // Keep only the latest kind:0 event per pubkey (the bridge does not
      // honor the `kinds` filter under search, and may return several
      // profile revisions — mirrors desktop's `list_user_search_results`).
      final latestByPubkey = <String, NostrEvent>{};
      for (final event in events) {
        if (event.kind != 0) continue;
        final pk = event.pubkey.toLowerCase();
        final current = latestByPubkey[pk];
        if (current == null || event.createdAt > current.createdAt) {
          latestByPubkey[pk] = event;
        }
      }

      return [
        for (final event in latestByPubkey.values) _profileFromEvent(event),
      ];
    });

UserProfile _profileFromEvent(NostrEvent event) {
  final data = ProfileData.fromEvent(event);
  return UserProfile(
    pubkey: event.pubkey.toLowerCase(),
    displayName: data.displayName,
    avatarUrl: data.avatarUrl,
    about: data.about,
    nip05Handle: data.nip05,
    ownerPubkey: verifiedOaOwnerPubkey(event.tags, event.pubkey),
  );
}

/// One chooser opening: an inline `@` token or one open picker.
typedef MentionChooserArgs = ({String channelId, String query, int opening});

/// The last finished directory search for each chooser opening. A new query
/// keeps showing its still-matching people while its own search runs.
/// Nothing here outlives the relay session, and a new opening starts empty.
class _SettledSearches {
  final _pages = <(String, int), List<UserProfile>>{};

  List<UserProfile>? last(String channelId, int opening) =>
      _pages[(channelId, opening)];

  /// Keeps [people] as this opening's last search. A search that found no
  /// one is not kept, so the next opening searches again.
  void settle(String channelId, int opening, List<UserProfile> people) {
    if (people.isEmpty) return;
    final key = (channelId, opening);
    _pages.remove(key);
    _pages[key] = people;
    if (_pages.length > 20) _pages.remove(_pages.keys.first);
  }
}

final _settledSearchesProvider = Provider<_SettledSearches>((ref) {
  ref.watch(relaySessionProvider.select((s) => s.status));
  return _SettledSearches();
});

/// Explicit mention choices per channel, newest highest. Memory only,
/// bounded to 100 channels and 100 keys each.
class MentionHistoryNotifier extends Notifier<Map<String, Map<String, int>>> {
  var _clock = 0;

  @override
  Map<String, Map<String, int>> build() {
    ref.watch(currentPubkeyProvider);
    return const {};
  }

  void remember(String channelId, String pubkey) {
    final channels = {...state};
    final keys = {...?channels.remove(channelId)};
    keys.remove(pubkey);
    keys[pubkey] = ++_clock;
    while (keys.length > 100) {
      keys.remove(keys.keys.first);
    }
    channels[channelId] = keys;
    while (channels.length > 100) {
      channels.remove(channels.keys.first);
    }
    state = channels;
  }
}

final mentionHistoryProvider =
    NotifierProvider<MentionHistoryNotifier, Map<String, Map<String, int>>>(
      MentionHistoryNotifier.new,
    );

/// Ranked mention candidates for a channel + query, by the portable mention
/// rules: channel members, the agents offered here, and (in streams and
/// forums only) people from a community directory search. A multi-word
/// query that continues no known name, including the names its search
/// found, is prose: it has no choices.
final mentionCandidatesProvider = Provider.family
    .autoDispose<List<MentionCandidate>, MentionChooserArgs>((ref, args) {
      final channelsAsync = ref.watch(channelsProvider);
      final membersAsync = ref.watch(channelMembersProvider(args.channelId));
      final sessionStatus = ref.watch(relaySessionProvider).status;
      final cachedMembers = channelsAsync.asData == null
          ? const <ChannelMember>[]
          : ref
                .read(channelsProvider.notifier)
                .cachedMembersForChannel(args.channelId);
      final members = channelMembersForAutocomplete(
        membersAsync: membersAsync,
        sessionStatus: sessionStatus,
        cachedMembers: cachedMembers,
      );
      final relayAgents =
          ref.watch(agentDirectoryProvider).asData?.value ??
          const <AgentDirectoryEntry>[];
      final owners = ref.watch(agentOwnersProvider).asData?.value ?? const {};
      final channels = channelsAsync.asData?.value ?? const <Channel>[];
      final userCache = ref.watch(userCacheProvider);
      final currentPubkey = ref.watch(currentPubkeyProvider);
      final channel = channels
          .where((candidate) => candidate.id == args.channelId)
          .firstOrNull;
      // Archived channels take no mentions. DMs never add directory people.
      if (channel?.isArchived == true) return const [];
      final directory =
          channel != null && (channel.isStream || channel.isForum);
      final settled = ref.watch(_settledSearchesProvider);

      final sharedChannelIds = {
        for (final channel in channels)
          if (channel.isMember && !channel.isArchived) channel.id,
      };

      List<MentionCandidate> build(List<UserProfile> searchResults) => [
        for (final candidate in buildMentionCandidates(
          members: members,
          relayAgents: relayAgents,
          sharedChannelIds: sharedChannelIds,
          userCache: userCache,
          ownerByAgentPubkey: owners,
          searchResults: searchResults,
          currentPubkey: currentPubkey,
        ))
          if (isMentionKey(candidate.pubkey)) candidate,
      ];

      // Admission does not gate the search: `@Mary J` can name a person
      // outside the channel, so the search runs and admission is checked
      // again with the people it finds (portable mention rules, section 2).
      var candidates = build(
        directory
            ? settled.last(args.channelId, args.opening) ??
                  const <UserProfile>[]
            : const <UserProfile>[],
      );
      if (directory) {
        final search = ref.watch(mentionUserSearchProvider(args.query));
        final page = search.asData?.value;
        if (page != null) {
          settled.settle(args.channelId, args.opening, page);
          candidates = build(page);
        }
      }
      if (!matchesMentionQuery(args.query, [
        for (final candidate in candidates)
          if (candidate.displayName?.trim().isNotEmpty == true)
            candidate.displayName!.trim(),
      ])) {
        return const [];
      }

      final names = mentionPickerNames(
        ref.watch(identityNameSourcesProvider),
        candidates,
      );
      loadIdentityNameOwners(ref, names);
      final presence = ref.watch(presenceCacheProvider);
      return rankMentionCandidates(
        [
          for (final candidate in candidates)
            candidate.withContextLabel(names.resolve(candidate.pubkey)?.name),
        ],
        args.query,
        viewer: currentPubkey,
        history: ref.watch(mentionHistoryProvider)[args.channelId] ?? const {},
        presence: (key) => presence[key] ?? 'unknown',
      );
    });

/// Picker labels compare every selectable choice, before query ranking
/// filters them, so a row's label does not change as the query narrows.
IdentityNames mentionPickerNames(
  IdentityNameSources sources,
  List<MentionCandidate> candidates,
) => sources.scope(
  [for (final candidate in candidates) candidate.pubkey],
  agentPubkeys: {
    for (final candidate in candidates)
      if (candidate.isAgent) candidate.pubkey,
  },
  fallbackNames: {
    for (final candidate in candidates)
      if (candidate.displayName?.trim().isNotEmpty == true)
        candidate.pubkey: candidate.displayName!,
  },
  // A newly searched agent can carry its verified owner before its profile
  // reaches the cache; that owner still decides mine-before-others.
  ownerPubkeys: {
    for (final candidate in candidates)
      candidate.pubkey: ?candidate.ownerPubkey,
  },
);
