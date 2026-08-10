/// Pubkeys tagged as message mentions, normalized for profile lookups.
Set<String> mentionedPubkeysFromTags(Iterable<List<String>> tags) => {
  for (final tag in tags)
    if (tag.length >= 2 && (tag[0] == 'p' || tag[0] == 'mention'))
      tag[1].toLowerCase(),
};
