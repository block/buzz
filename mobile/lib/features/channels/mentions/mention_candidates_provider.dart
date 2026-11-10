import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../../shared/agents/owned_agent_directory.dart';
import '../../../shared/crypto/nip_oa.dart';
import '../../../shared/identity_names/identity_names.dart';
import '../../../shared/identity_names/identity_names_provider.dart';
import '../../../shared/mentions/agent_identity_provider.dart';
import '../../../shared/relay/relay.dart';
import '../../../shared/profile/user_cache_provider.dart';
import '../../../shared/profile/user_profile.dart';
import '../../../shared/verified_profiles/verified_profile.dart';
import '../../../shared/verified_profiles/verified_profile_directory.dart';
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

/// Global user search for mention autocomplete — profile name search via
/// the relay HTTP bridge. Mirrors desktop's `useInfiniteUserSearchQuery`
/// feeding `useMentions` (source 4: people and agents outside the channel).
///
/// Debounced: the provider waits [mentionSearchDebounce] before querying;
/// keystrokes dispose the stale family member so its request never fires.
/// Each finished search is kept in [mentionSearchCacheProvider].
final mentionUserSearchProvider = FutureProvider.autoDispose
    .family<List<UserProfile>, String>((ref, query) async {
      // The session notifier is stable across communities, and two
      // communities can share a signing key. Watching the relay config
      // restarts the search at a community switch, so a late result or
      // error from the old community never reaches the new one.
      ref.watch(relayConfigProvider);
      final trimmed = query.trim();
      if (trimmed.isEmpty) return const [];

      var disposed = false;
      ref.onDispose(() => disposed = true);
      await Future<void>.delayed(mentionSearchDebounce);
      if (disposed) return const [];

      final session = ref.read(relaySessionProvider.notifier);
      final events = await session.searchProfiles(trimmed);

      // Each person once, by their newest kind:0; someone with none, by
      // their verified head.
      final found = await ref
          .read(verifiedProfilesProvider.notifier)
          .searched(events);
      final people = [
        for (final person in found)
          if (person.profile case final profile?)
            _profileFromEvent(profile)
          else
            UserProfile(
              pubkey: person.pubkey,
              displayName: person.verified?.displayName,
            ),
      ];
      // A search that outlived its query or community is not kept.
      if (!disposed) ref.read(mentionSearchCacheProvider).put(trimmed, people);
      return people;
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

/// Finished directory searches by query, for the current community and
/// signer. A query searched before shows its people at once while a fresh
/// search runs, so typing the same query twice shows the same rows. A
/// query not searched yet starts from the longest searched query it
/// continues: a name search for `sa` finds no one that `s` did not.
class MentionSearchCache {
  static const limit = 64;
  final _pages = <String, List<UserProfile>>{};

  static String _key(String query) => query.trim().toLowerCase();

  /// The finished search for exactly [query], or null.
  List<UserProfile>? exact(String query) => _pages[_key(query)];

  /// The finished search for [query], else the one for the longest
  /// searched query that [query] continues, else null.
  List<UserProfile>? seed(String query) {
    final key = _key(query);
    if (key.isEmpty) return null;
    for (var end = key.length; end > 0; end--) {
      final page = _pages[key.substring(0, end)];
      if (page != null) return page;
    }
    return null;
  }

  /// Keeps [people] as the latest search for [query], newest last.
  void put(String query, List<UserProfile> people) {
    final key = _key(query);
    if (key.isEmpty) return;
    _pages.remove(key);
    _pages[key] = people;
    while (_pages.length > limit) {
      _pages.remove(_pages.keys.first);
    }
  }
}

/// Nothing in the search cache outlives the community or the signer.
final mentionSearchCacheProvider = Provider<MentionSearchCache>((ref) {
  ref.watch(relayConfigProvider);
  ref.watch(currentPubkeyProvider);
  return MentionSearchCache();
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
/// rules: channel members, the agents offered here, and (in streams, forums
/// and DMs) people from a community directory search. A multi-word
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
      // Archived channels take no mentions.
      final channel = ref.watch(channelByIdProvider(args.channelId));
      if (channel?.isArchived == true) return const [];
      return _chooserCandidates(
        ref,
        members: members,
        query: args.query,
        directory: channel?.mentionsOutsidePeople ?? false,
        // Desktop offers the owner's agents where they can be added.
        offerOwnedAgents:
            channel != null && (channel.isStream || channel.isForum),
        historyKey: args.channelId,
      );
    });

/// [mentionHistoryProvider] key for choices made in the New DM picker.
const newDmMentionHistoryKey = 'new-dm';

/// [mentionHistoryProvider] key for choices made in an Add members picker.
const addMembersMentionHistoryKey = 'add-members';

/// One recipient picker search: its [query], the [historyKey] its choices
/// are remembered under, whether agents that do not answer the viewer
/// are offered too ([unsharedAgents]), as a channel can hold them, and the
/// channel whose members the picker hides ([channelId]), if any.
typedef RecipientSearch = ({
  String query,
  String historyKey,
  bool unsharedAgents,
  String? channelId,
});

/// Ranked recipients for a picker (New DM, Add members), chosen exactly like
/// mention autocomplete in a stream: no channel members yet, the viewer's
/// own agents offered unprompted, agents shared with the viewer, and people
/// from the community directory search, with the same debounce and search
/// cache. Before a name is typed, where `@` lists the channel's members, a
/// picker lists its agents and then the people the viewer shares a channel
/// or DM with, so known people never push an agent off the list.
final recipientCandidatesProvider = Provider.family
    .autoDispose<List<MentionCandidate>, RecipientSearch>((ref, search) {
      final currentPubkey = ref.watch(currentPubkeyProvider)?.toLowerCase();
      final empty = search.query.trim().isEmpty;
      final candidates = [
        for (final candidate in _chooserCandidates(
          ref,
          members: const [],
          query: search.query,
          directory: true,
          offerOwnedAgents: true,
          offerUnsharedAgents: search.unsharedAgents,
          historyKey: search.historyKey,
          knownPeople: empty
              ? _knownPeople(ref, excludedChannelId: search.channelId)
              : const [],
        ))
          if (candidate.pubkey != currentPubkey) candidate,
      ];
      if (!empty) return candidates;
      return [
        for (final candidate in candidates)
          if (candidate.isAgent) candidate,
        for (final candidate in candidates)
          if (!candidate.isAgent) candidate,
      ];
    });

/// Unnamed known people whose profiles a picker opening already requested,
/// so a person with no profile is asked for once, not on every rebuild.
final _requestedKnownPeopleProvider = Provider.autoDispose<Set<String>>(
  (ref) => <String>{},
);

/// Whether a recipient picker's directory search for [query] still runs.
final recipientSearchPendingProvider = Provider.family
    .autoDispose<bool, String>(
      (ref, query) =>
          query.trim().isNotEmpty &&
          ref.watch(mentionUserSearchProvider(query)).isLoading,
    );

/// Whether a recipient picker's directory search for [query] failed.
final recipientSearchFailedProvider = Provider.family.autoDispose<bool, String>(
  (ref, query) {
    final search = ref.watch(mentionUserSearchProvider(query));
    return search.hasError && !search.isLoading;
  },
);

/// Most roster entries one known-people scan reads.
const _knownPeopleScanLimit = 2000;

/// Named people in the viewer's open channels and DMs, as directory search
/// results, from the member lists the channel list already loaded. DM
/// partners come first, and members of [excludedChannelId] are skipped. At
/// most [mentionSuggestionLimit] people are gathered, a full picker list;
/// profiles are loaded for the unnamed ones among them.
List<UserProfile> _knownPeople(Ref ref, {String? excludedChannelId}) {
  final listed = ref.watch(channelsProvider).asData?.value ?? const [];
  final channels = [
    for (final channel in listed)
      if (channel.isDm) channel,
    for (final channel in listed)
      if (!channel.isDm) channel,
  ];
  final notifier = ref.read(channelsProvider.notifier);
  final userCache = ref.watch(userCacheProvider);
  final verified = ref.watch(verifiedProfilesProvider);
  final viewer = ref.watch(currentPubkeyProvider)?.toLowerCase();
  final seen = <String>{
    ?viewer,
    if (excludedChannelId != null)
      for (final member in notifier.cachedMembersForChannel(excludedChannelId))
        member.pubkey.toLowerCase(),
  };
  final people = <UserProfile>[];
  final unnamed = <String>[];
  // Roster entries read, bounded even when most are already excluded.
  var visited = 0;
  gather:
  for (final channel in channels) {
    if (!channel.isMember || channel.isArchived) continue;
    if (channel.id == excludedChannelId) continue;
    final pubkeys = channel.participantPubkeys.followedBy(
      notifier.cachedMembersForChannel(channel.id).map((m) => m.pubkey),
    );
    for (final pubkey in pubkeys) {
      if (people.length + unnamed.length >= mentionSuggestionLimit ||
          ++visited > _knownPeopleScanLimit) {
        break gather;
      }
      final key = pubkey.toLowerCase();
      if (!seen.add(key)) continue;
      final profile = userCache[key];
      if (profile?.displayName?.trim().isNotEmpty == true) {
        people.add(profile!);
      } else if (verified[key] case final name?) {
        people.add(UserProfile(pubkey: key, displayName: name.displayName));
      } else {
        unnamed.add(key);
      }
    }
  }
  final requested = ref.watch(_requestedKnownPeopleProvider);
  final load = [
    for (final pubkey in unnamed)
      if (requested.add(pubkey)) pubkey,
  ];
  if (load.isNotEmpty) {
    Future.microtask(() {
      if (ref.mounted) ref.read(userCacheProvider.notifier).preload(load);
    });
  }
  return people;
}

List<MentionCandidate> _chooserCandidates(
  Ref ref, {
  required List<ChannelMember> members,
  required String query,
  required bool directory,
  required bool offerOwnedAgents,
  bool offerUnsharedAgents = false,
  required String historyKey,
  List<UserProfile> knownPeople = const [],
}) {
  final channels =
      ref.watch(channelsProvider).asData?.value ?? const <Channel>[];
  final relayAgents =
      ref.watch(agentDirectoryProvider).asData?.value ??
      const <AgentDirectoryEntry>[];
  final owners = ref.watch(agentOwnersProvider).asData?.value ?? const {};
  // Null while loading or failed: nothing is known to be unlisted.
  final ownedAgents = ref.watch(ownedAgentDirectoryProvider).asData?.value;
  final userCache = ref.watch(userCacheProvider);
  final currentPubkey = ref.watch(currentPubkeyProvider);
  final searches = ref.watch(mentionSearchCacheProvider);

  final sharedChannelIds = {
    for (final channel in channels)
      if (channel.isMember && !channel.isArchived) channel.id,
  };

  // The directory search matches self-declared names only. A person
  // outside the channel whose cached verified name matches joins the
  // results, so the name headers show finds them, not only an impostor
  // who copied it into kind 0.
  final verified = ref.watch(verifiedProfilesProvider);
  final verifiedMatches = directory
      ? verifiedMentionMatches(verified, query, userCache)
      : const <UserProfile>[];

  List<MentionCandidate> build(List<UserProfile> searchResults) => [
    for (final candidate in buildMentionCandidates(
      members: members,
      relayAgents: relayAgents,
      sharedChannelIds: sharedChannelIds,
      userCache: userCache,
      ownerByAgentPubkey: owners,
      searchResults: [...searchResults, ...verifiedMatches, ...knownPeople],
      currentPubkey: currentPubkey,
      ownedAgents: ownedAgents,
      offerOwnedAgents: offerOwnedAgents,
      offerUnsharedAgents: offerUnsharedAgents,
    ))
      if (isMentionKey(candidate.pubkey)) candidate,
  ];

  // Admission does not gate the search: `@Mary J` can name a person
  // outside the channel, so the search runs and admission is checked
  // again with the people it finds (portable mention rules, section 2).
  var candidates = build(
    directory
        ? searches.seed(query) ?? const <UserProfile>[]
        : const <UserProfile>[],
  );
  if (directory) {
    final search = ref.watch(mentionUserSearchProvider(query));
    final page = search.asData?.value;
    if (page != null) candidates = build(page);
  }
  // A verified name replaces the self-declared one for matching, ranking
  // and the inserted text, so typing it finds the verified person.
  candidates = [
    for (final candidate in candidates)
      switch (verified[candidate.pubkey.toLowerCase()]) {
        final profile? => candidate.withVerifiedName(profile),
        null => candidate,
      },
  ];
  if (!matchesMentionQuery(query, [
    for (final candidate in candidates) ...candidate.searchNames,
  ])) {
    return const [];
  }

  final names = mentionPickerNames(
    ref.watch(identityNameSourcesProvider),
    candidates,
  );
  loadIdentityNameOwners(ref, names);
  final presence = ref.watch(presentedPresenceProvider);
  return rankMentionCandidates(
    [
      for (final candidate in candidates)
        candidate.withContextLabel(names.labelFor(candidate.pubkey)),
    ],
    query,
    viewer: currentPubkey,
    history: ref.watch(mentionHistoryProvider)[historyKey] ?? const {},
    presence: (key) => presence[key] ?? 'unknown',
  );
}

/// People whose verified name matches a non-empty [query], as directory
/// search results. Their cached kind 0 profile is kept when known.
List<UserProfile> verifiedMentionMatches(
  Map<String, VerifiedProfile> verified,
  String query,
  Map<String, UserProfile> userCache,
) {
  if (query.trim().isEmpty) return const [];
  return [
    for (final MapEntry(:key, :value) in verified.entries)
      if (rankMentions([
        MentionChoice(
          pubkey: key,
          name: value.displayName,
          label: value.displayName,
          aliases: [value.displayName, ...verifiedAliases(value)],
          member: false,
        ),
      ], query).isNotEmpty)
        userCache[key] ??
            UserProfile(pubkey: key, displayName: value.displayName),
  ];
}

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

/// Whether the directory search for this chooser failed. The chooser keeps
/// the error and a retry instead of closing (portable mention rules,
/// section 2). Archived channels never search.
final mentionSearchFailedProvider = Provider.family
    .autoDispose<bool, MentionChooserArgs>((ref, args) {
      final channel = ref.watch(channelByIdProvider(args.channelId));
      if (channel == null ||
          channel.isArchived ||
          !channel.mentionsOutsidePeople) {
        return false;
      }
      final search = ref.watch(mentionUserSearchProvider(args.query));
      return search.hasError && !search.isLoading;
    });

/// Whether this chooser's directory search is still running, so people may
/// still join its rows. A cached query counts too: its refresh can add people
/// or the retry row below the cached page.
final mentionSearchPendingProvider = Provider.family
    .autoDispose<bool, MentionChooserArgs>((ref, args) {
      final channel = ref.watch(channelByIdProvider(args.channelId));
      if (channel == null ||
          channel.isArchived ||
          !channel.mentionsOutsidePeople ||
          args.query.trim().isEmpty) {
        return false;
      }
      return ref.watch(mentionUserSearchProvider(args.query)).isLoading;
    });
