import '../../../shared/mentions/mention_tags.dart';

/// Whether [tags] alone earn attention: the message mentions the viewer.
/// A reply also sent to the channel earns attention only in one of the
/// viewer's threads, which tags cannot tell (see [isInViewerThread]).
bool isHighPriorityEvent(List<List<String>> tags, String currentPubkey) =>
    tagsAddress(tags, currentPubkey);
