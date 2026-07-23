/// Case-insensitive display-name counts for the currently visible suggestions.
Map<String, int> countVisibleMentionDisplayNames(
  Iterable<String> displayNames,
) {
  final counts = <String, int>{};
  for (final displayName in displayNames) {
    final normalizedName = displayName.trim().toLowerCase();
    if (normalizedName.isEmpty) continue;
    counts[normalizedName] = (counts[normalizedName] ?? 0) + 1;
  }
  return counts;
}

bool hasVisibleMentionDisplayNameCollision(
  String displayName,
  Map<String, int> counts,
) {
  return (counts[displayName.trim().toLowerCase()] ?? 0) > 1;
}

/// Adds an agent's owner only when its name collides with another visible
/// suggestion. The original candidate label remains unchanged for insertion.
String formatDisambiguatedMentionDisplayName({
  required String displayName,
  required bool hasNameCollision,
  required bool isAgent,
  required String? ownerLabel,
}) {
  final normalizedOwnerLabel = ownerLabel?.trim();
  if (!hasNameCollision ||
      !isAgent ||
      normalizedOwnerLabel == null ||
      normalizedOwnerLabel.isEmpty) {
    return displayName;
  }
  return '$displayName ($normalizedOwnerLabel)';
}
