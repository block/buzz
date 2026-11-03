/// Resolve NIP-10 markers using relay ingest validity and ordering rules.
///
/// The last valid 64-character ASCII-hex marker wins. IDs are lowercased for
/// client lookup. Root-only events remain top-level; a reply without a valid
/// root uses its parent as root.
({String? parentId, String? rootId}) parseThreadReference(
  List<List<String>> tags,
) {
  String? parentId;
  String? rootId;
  for (final tag in tags) {
    if (tag.length < 4 || tag[0] != 'e') continue;
    final id = tag[1];
    if (id.length != 64 || !_threadEventId.hasMatch(id)) continue;
    if (tag[3] == 'reply') parentId = id.toLowerCase();
    if (tag[3] == 'root') rootId = id.toLowerCase();
  }
  if (parentId == null) return (parentId: null, rootId: null);
  return (parentId: parentId, rootId: rootId ?? parentId);
}

final _threadEventId = RegExp(r'^[0-9a-fA-F]{64}$');
