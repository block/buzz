/// Pubkeys tagged as message mentions, normalized for profile lookups.
Set<String> mentionedPubkeysFromTags(Iterable<List<String>> tags) => {
  for (final tag in tags)
    if (tag.length >= 2 && (tag[0] == 'p' || tag[0] == 'mention'))
      tag[1].toLowerCase(),
};

final _pubkeyHex = RegExp(r'^[0-9a-f]{64}$');

/// Whether [pubkey] is shaped like a Nostr pubkey: 64 hex characters.
///
/// The `p` tag naming the author a reply answers is what delivers that reply to
/// the agent being answered, so a malformed value costs the delivery instead of
/// failing loudly. The Rust builder rejects one in `check_pubkey` and the
/// desktop builder drops it before tagging; this is the same gate for the
/// mobile builders.
bool isPubkeyShaped(String? pubkey) =>
    pubkey != null && _pubkeyHex.hasMatch(pubkey.toLowerCase());
