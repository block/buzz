import '../../../shared/mentions/mention_tags.dart';

bool isHighPriorityEvent(List<List<String>> tags, String currentPubkey) {
  return tagsAddress(tags, currentPubkey) ||
      tags.any(
        (tag) => tag.length >= 2 && tag[0] == 'broadcast' && tag[1] == '1',
      );
}
