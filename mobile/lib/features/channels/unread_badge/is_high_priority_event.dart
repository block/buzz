bool isHighPriorityEvent(List<List<String>> tags, String currentPubkey) {
  final normalizedPubkey = currentPubkey.toLowerCase();
  for (final tag in tags) {
    if (tag.length >= 2 &&
        tag[0] == 'p' &&
        tag[1].toLowerCase() == normalizedPubkey) {
      return true;
    }
    if (tag.length >= 2 && tag[0] == 'broadcast' && tag[1] == '1') {
      return true;
    }
  }
  return false;
}
