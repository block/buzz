import 'nostr_models.dart';

/// Aggregate cap on explicit `#h` channel values the relay accepts across one
/// REQ's filters before rejecting it as `restricted: too many explicit
/// channels`. Mirrors `MAX_EXPLICIT_CHANNEL_VALUES` in
/// `crates/buzz-relay/src/handlers/req.rs`. A subscription over more channels
/// than this must be split into multiple REQs, each within the cap.
const int kMaxExplicitChannelValues = 128;

/// Canonical [NostrFilter] constructors for common Buzz queries.
///
/// Centralising filter shapes keeps relay queries consistent across providers
/// and makes kind/tag conventions easy to audit.
abstract final class NostrFilters {
  /// Channel metadata for the given channel IDs.
  static NostrFilter channelMetadata(List<String> ids) =>
      NostrFilter(kinds: [39000], tags: {'#d': ids}, limit: ids.length);

  /// Members list for a single channel.
  static NostrFilter channelMembers(String channelId) => NostrFilter(
    kinds: [39002],
    tags: {
      '#d': [channelId],
    },
    limit: 1,
  );

  /// A single user's profile (kind:0).
  static NostrFilter profile(String pubkey) =>
      NostrFilter(kinds: [0], authors: [pubkey], limit: 1);

  /// Batch user profiles (kind:0) for multiple pubkeys.
  static NostrFilter profilesBatch(List<String> pubkeys) =>
      NostrFilter(kinds: [0], authors: pubkeys, limit: pubkeys.length);

  /// Channel messages (all event kinds that appear in channels).
  static NostrFilter messages(
    String channelId, {
    int limit = 200,
    int? until,
    String? beforeId,
  }) => NostrFilter(
    kinds: EventKind.channelEventKinds,
    tags: {
      '#h': [channelId],
    },
    limit: limit,
    until: until,
    extensions: {'before_id': ?beforeId},
  );

  /// A channel's system notices (kind:40099), newest first, below the
  /// `until` + `before_id` cursor the channel-window pages use.
  static NostrFilter channelNotices(
    String channelId, {
    int? until,
    String? beforeId,
  }) => NostrFilter(
    kinds: const [EventKind.systemMessage],
    tags: {
      '#h': [channelId],
    },
    limit: channelNoticePageSize,
    until: until,
    extensions: {'before_id': ?beforeId},
  );

  /// Notices one [channelNotices] read asks for.
  static const channelNoticePageSize = 500;

  /// Reactions and deletions that target [eventIds]: the auxiliary events a
  /// channel-window page returns with its rows (`include_aux`), newest first,
  /// below the `until` + `before_id` cursor. Call again with the reaction ids,
  /// asking only for [EventKind.deletion] and [EventKind.nip29DeleteEvent],
  /// to read removed reactions.
  ///
  /// With [channelId], the read names the channel in `#h`: a system channel,
  /// or an unlisted one for a non-member, shows its events only to a filter
  /// that names it (NIP-CL). Without it, the read also finds deletions stored
  /// with no channel.
  static NostrFilter channelAux(
    List<String> eventIds, {
    String? channelId,
    List<int> kinds = channelAuxKinds,
    int? until,
    String? beforeId,
  }) => NostrFilter(
    kinds: kinds,
    tags: {
      '#e': eventIds,
      if (channelId != null) '#h': [channelId],
    },
    limit: channelAuxLimit,
    until: until,
    extensions: {'before_id': ?beforeId},
  );

  /// The kinds [channelAux] reads by default.
  static const channelAuxKinds = [
    EventKind.deletion,
    EventKind.reaction,
    EventKind.nip29DeleteEvent,
  ];

  /// Auxiliary events one [channelAux] read asks for.
  static const channelAuxLimit = 1000;

  /// Huddle lifecycle state for one parent channel, independent of timeline paging.
  static NostrFilter huddleLifecycle(String channelId) => NostrFilter(
    kinds: [EventKind.huddleStarted, EventKind.huddleEnded],
    tags: {
      '#h': [channelId],
    },
    limit: 200,
  );

  /// Canvas event for a channel.
  static NostrFilter canvas(String channelId) => NostrFilter(
    kinds: [40100],
    tags: {
      '#h': [channelId],
    },
    limit: 1,
  );

  /// Latest per-viewer hidden-DM snapshot (kind:30622, `#p` = my pubkey).
  static NostrFilter hiddenDms(String myPk) => NostrFilter(
    kinds: [EventKind.dmVisibility],
    tags: {
      '#p': [myPk],
    },
    limit: 1,
  );

  /// Forum posts (kind:45001) in a channel.
  static NostrFilter forumPosts(
    String channelId, {
    int limit = 50,
    int? until,
  }) => NostrFilter(
    kinds: [45001],
    tags: {
      '#h': [channelId],
    },
    limit: limit,
    until: until,
  );

  /// Replies in a forum thread (root event id + channel scope).
  static NostrFilter forumThread(String rootId, String channelId) =>
      NostrFilter(
        kinds: [9, 45003],
        tags: {
          '#e': [rootId],
          '#h': [channelId],
        },
      );

  /// Global user search over kind:0 profiles (NIP-50 via the HTTP bridge).
  ///
  /// `search_mode: "prefix"` is a Buzz bridge-only extension: every caller is
  /// a typeahead surface, so a partially typed name must match ("rac" →
  /// "raccoon"). Mirrors desktop's `build_user_search_filter`
  /// (desktop/src-tauri/src/commands/profile.rs). Bridge-only — send through
  /// `queryRelay`, not a WebSocket REQ.
  static NostrFilter searchUsers(String query, {int limit = 50}) => NostrFilter(
    kinds: [0],
    search: query,
    limit: limit,
    extensions: const {'search_mode': 'prefix'},
  );

  /// Relay membership list (kind:13534).
  static NostrFilter relayMembers() =>
      const NostrFilter(kinds: [EventKind.relayMembership], limit: 1);

  /// Agent profiles (kind:10100).
  static NostrFilter agentProfiles() =>
      const NostrFilter(kinds: [10100], limit: 100);
}
