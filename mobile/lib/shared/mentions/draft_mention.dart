/// A device-local exact selection. Display classification is not authorization;
/// membership and owner policy must still be checked when sending.
class DraftMention {
  final String pubkey;
  final bool isAgent;
  const DraftMention({required this.pubkey, this.isAgent = false});
  Map<String, Object> toJson() => {'pubkey': pubkey, 'is_agent': isAgent};

  /// Ignore malformed legacy entries rather than guessing an identity.
  static Map<String, DraftMention> decode(Object? raw) {
    if (raw is! Map) return const {};
    final result = <String, DraftMention>{};
    for (final entry in raw.entries) {
      if (entry.key is! String || entry.value is! Map) continue;
      final label = entry.key as String;
      final key = entry.value['pubkey'];
      if (label.isEmpty ||
          key is! String ||
          !RegExp(r'^[0-9a-fA-F]{64}$').hasMatch(key)) {
        continue;
      }
      result[label] = DraftMention(
        pubkey: key.toLowerCase(),
        isAgent: entry.value['is_agent'] == true,
      );
    }
    return result;
  }
}
