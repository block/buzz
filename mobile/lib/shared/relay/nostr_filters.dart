import 'nostr_models.dart';

/// Canonical [NostrFilter] constructors for common Buzz queries.
///
/// Centralising filter shapes keeps relay queries consistent across providers
/// and makes kind/tag conventions easy to audit.
abstract final class NostrFilters {
  /// Channels where I'm a member (kind:39002 with `#p` = my pubkey).
  static NostrFilter myChannels(String myPk) => NostrFilter(
    kinds: [39002],
    tags: {
      '#p': [myPk],
    },
    limit: 500,
  );

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
  }) => NostrFilter(
    kinds: EventKind.channelEventKinds,
    tags: {
      '#h': [channelId],
    },
    limit: limit,
    until: until,
  );

  /// Reactions (kind:7) on a specific event.
  static NostrFilter reactions(String eventId) => NostrFilter(
    kinds: [7],
    tags: {
      '#e': [eventId],
    },
  );

  /// Canvas event for a channel.
  static NostrFilter canvas(String channelId) => NostrFilter(
    kinds: [40100],
    tags: {
      '#h': [channelId],
    },
    limit: 1,
  );

  /// Workflows (kind:30620) in a channel.
  static NostrFilter workflows(String channelId) => NostrFilter(
    kinds: [30620],
    tags: {
      '#h': [channelId],
    },
  );

  /// DM channels where I'm a participant.
  static NostrFilter dmList(String myPk) => NostrFilter(
    kinds: [39000],
    tags: {
      '#t': ['dm'],
      '#p': [myPk],
    },
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

  /// NIP-50 message search, optionally scoped to a channel.
  static NostrFilter searchMessages(
    String query, {
    String? channelId,
    int limit = 20,
  }) => NostrFilter(
    kinds: [9, 40002, 45001, 45003],
    tags: channelId != null
        ? {
            '#h': [channelId],
          }
        : const {},
    search: query,
    limit: limit,
  );

  /// Deletions (kind:5) targeting event IDs.
  static NostrFilter deletionsByTargetIds(
    List<String> ids, {
    List<String>? authors,
  }) => NostrFilter(
    kinds: [EventKind.deletion],
    authors: authors,
    tags: {'#e': ids},
    limit: ids.length,
  );

  /// User notes (kind:1) for the global Pulse timeline.
  static NostrFilter globalNotes({int limit = 50, int? until}) =>
      NostrFilter(kinds: [EventKind.note], limit: limit, until: until);

  /// Notes by a set of authors for Pulse timelines.
  static NostrFilter notesTimeline(
    List<String> pubkeys, {
    int limit = 200,
    int? until,
  }) => NostrFilter(
    kinds: [EventKind.note],
    authors: pubkeys,
    limit: limit,
    until: until,
  );

  /// Reactions authored by a user.
  static NostrFilter userReactions(String pubkey, {int limit = 200}) =>
      NostrFilter(kinds: [EventKind.reaction], authors: [pubkey], limit: limit);

  /// Reactions targeting notes.
  static NostrFilter noteReactions(List<String> noteIds) => NostrFilter(
    kinds: [EventKind.reaction],
    tags: {'#e': noteIds},
    limit: 500,
  );

  /// Fetch notes by ids.
  static NostrFilter notesByIds(List<String> ids) =>
      NostrFilter(kinds: [EventKind.note], ids: ids, limit: ids.length);

  /// User notes (kind:1) for a single author.
  static NostrFilter userNotes(String pubkey, {int limit = 20, int? until}) =>
      NostrFilter(
        kinds: [EventKind.note],
        authors: [pubkey],
        limit: limit,
        until: until,
      );

  /// Contact list (kind:3) for a user.
  static NostrFilter contactList(String pubkey) =>
      NostrFilter(kinds: [EventKind.contactList], authors: [pubkey], limit: 1);

  /// Relay membership list (kind:13534).
  static NostrFilter relayMembers() =>
      const NostrFilter(kinds: [13534], limit: 1);

  /// Agent profiles (kind:10100).
  static NostrFilter agentProfiles() =>
      const NostrFilter(kinds: [10100], limit: 100);

  /// User status (NIP-38, kind:30315).
  static NostrFilter userStatus(String pubkey) =>
      NostrFilter(kinds: [30315], authors: [pubkey], limit: 1);
}
