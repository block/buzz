import 'package:flutter/foundation.dart';

/// A mention autocomplete candidate. Mirrors the desktop's
/// `MentionCandidateForRanking` (desktop/src/features/messages/lib/mentionRanking.ts).
@immutable
class MentionCandidate {
  final String pubkey;
  final String? displayName;
  final String? secondaryLabel;
  final String? avatarUrl;
  final bool isAgent;
  final bool isMember;
  final String? role;
  final String? ownerPubkey;

  const MentionCandidate({
    required this.pubkey,
    this.displayName,
    this.secondaryLabel,
    this.avatarUrl,
    this.isAgent = false,
    this.isMember = false,
    this.role,
    this.ownerPubkey,
  });

  String get label {
    final name = displayName?.trim();
    if (name != null && name.isNotEmpty) return name;
    return pubkey.length >= 8 ? pubkey.substring(0, 8) : pubkey;
  }
}

/// Group rank: channel members, then people, then other agents.
/// Mirrors desktop's `getMentionCandidateGroupRank` (personas are a
/// desktop-only concept; their slot between members and people is unused
/// here so numbering stays aligned).
int _groupRank(MentionCandidate candidate) {
  if (candidate.isMember) return 0;
  if (!candidate.isAgent) return 2;
  return 3;
}

/// Match-quality score for one label. Lower is better; null means no match.
/// Mirrors desktop's `scoreMentionCandidateLabel`.
int? _scoreLabel(String label, String lowerQuery) {
  final lower = label.toLowerCase();
  if (lower == lowerQuery) return 0;
  if (lower.startsWith(lowerQuery)) return 1;

  final words = lower.split(RegExp(r'[\s\-_]+')).where((w) => w.isNotEmpty);
  if (words.any((word) => word == lowerQuery)) return 2;
  if (words.any((word) => word.startsWith(lowerQuery))) return 3;

  return null;
}

/// Rank candidates for a mention query. Mirrors desktop's
/// `rankMentionCandidates`: sort by group, then match quality, then the
/// stable original order.
List<MentionCandidate> rankMentionCandidates(
  List<MentionCandidate> candidates,
  String query, {
  String? currentPubkey,
}) {
  final lowerQuery = query.toLowerCase();
  final currentLower = currentPubkey?.toLowerCase();

  final ranked = <(MentionCandidate, bool, int, int, int)>[];
  for (var order = 0; order < candidates.length; order++) {
    final candidate = candidates[order];

    final labelScores = [candidate.displayName, candidate.secondaryLabel].map((
      value,
    ) {
      final trimmed = value?.trim();
      if (trimmed == null || trimmed.isEmpty) return null;
      return _scoreLabel(trimmed, lowerQuery);
    }).whereType<int>();
    int? score = labelScores.isEmpty
        ? null
        : labelScores.reduce((a, b) => a < b ? a : b);

    if (score == null) {
      final pubkeyLower = candidate.pubkey.toLowerCase();
      if (pubkeyLower.startsWith(lowerQuery)) {
        score = 4;
      } else if (pubkeyLower.contains(lowerQuery)) {
        score = 5;
      }
    }

    if (score == null) continue;
    final isOwnedByCurrentUser =
        candidate.isAgent &&
        currentLower != null &&
        candidate.ownerPubkey?.toLowerCase() == currentLower;
    ranked.add((
      candidate,
      isOwnedByCurrentUser,
      _groupRank(candidate),
      score,
      order,
    ));
  }

  ranked.sort((a, b) {
    final owned = (b.$2 ? 1 : 0).compareTo(a.$2 ? 1 : 0);
    if (owned != 0) return owned;
    final group = a.$3.compareTo(b.$3);
    if (group != 0) return group;
    final score = a.$4.compareTo(b.$4);
    if (score != 0) return score;
    return a.$5.compareTo(b.$5);
  });

  return [for (final item in ranked) item.$1];
}
