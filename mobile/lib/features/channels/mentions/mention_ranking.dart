import 'package:flutter/foundation.dart';

import '../../../shared/mentions/mention_rules.dart';
import '../../../shared/utils/string_utils.dart';

/// A mention autocomplete candidate. Mirrors the desktop's
/// `MentionCandidateForRanking` (desktop/src/features/messages/lib/mentionRanking.ts).
@immutable
class MentionCandidate {
  final String pubkey;

  /// Restored identity only: eligibility must come from current community state.
  final bool requiresRevalidation;
  final String? displayName;
  final String? secondaryLabel;
  final String? avatarUrl;
  final bool isAgent;
  final bool isMember;
  final String? role;
  final String? ownerPubkey;

  /// Contextual identity label for the picker row. Presentation only: the
  /// inserted mention text still uses [label] and binds the exact [pubkey].
  final String? contextLabel;

  const MentionCandidate({
    required this.pubkey,
    this.requiresRevalidation = false,
    this.displayName,
    this.secondaryLabel,
    this.avatarUrl,
    this.isAgent = false,
    this.isMember = false,
    this.role,
    this.ownerPubkey,
    this.contextLabel,
  });

  /// The row label shown in the picker.
  String get pickerLabel => contextLabel ?? label;

  MentionCandidate withContextLabel(String? contextLabel) => MentionCandidate(
    pubkey: pubkey,
    requiresRevalidation: requiresRevalidation,
    displayName: displayName,
    secondaryLabel: secondaryLabel,
    avatarUrl: avatarUrl,
    isAgent: isAgent,
    isMember: isMember,
    role: role,
    ownerPubkey: ownerPubkey,
    contextLabel: contextLabel,
  );

  String get label {
    final name = displayName?.trim();
    if (name != null && name.isNotEmpty) return name;
    return shortPubkey(pubkey);
  }

  /// Avatar initial: display-name-derived, or keyed to the hex public key
  /// so unnamed identities keep distinct initials (a compact npub would
  /// render `N` for everyone).
  String get initial {
    final name = displayName?.trim();
    if (name != null && name.isNotEmpty) return name[0].toUpperCase();
    return pubkey.isNotEmpty ? pubkey[0].toUpperCase() : '?';
  }
}

/// The portable rule inputs for [candidate]. The label is what the chooser
/// shows; only a real display name is searchable, never a key or NIP-05.
MentionChoice mentionChoiceOf(MentionCandidate candidate, String? viewer) {
  final name = candidate.displayName?.trim() ?? '';
  return MentionChoice(
    pubkey: candidate.pubkey,
    name: name,
    label: candidate.label,
    aliases: name.isEmpty ? const [] : [name],
    member: candidate.isMember,
    agent: candidate.isAgent,
    owned:
        candidate.isAgent &&
        viewer != null &&
        candidate.ownerPubkey?.toLowerCase() == viewer.toLowerCase(),
  );
}

/// Rank candidates for a mention query with the portable mention rules
/// (sections 3 and 4): members first, then match quality, then the viewer's
/// own agents, then labels. [history] holds explicit choices in this channel
/// (higher is newer); [presence] maps a key to `online` or `away`.
List<MentionCandidate> rankMentionCandidates(
  List<MentionCandidate> candidates,
  String query, {
  String? viewer,
  Map<String, int> history = const {},
  String Function(String pubkey)? presence,
}) {
  final byKey = {for (final c in candidates) c.pubkey: c};
  return [
    for (final choice in rankMentions(
      [for (final c in byKey.values) mentionChoiceOf(c, viewer)],
      query,
      history: history,
      presence: presence,
    ))
      byKey[choice.pubkey]!,
  ];
}

/// Most rows one chooser shows. The user types more to narrow the list.
const mentionSuggestionLimit = 50;

/// The chooser rows for [ranked] when [previous] rows are already shown for
/// the same query and opening (section 6). Shown rows keep their places and
/// take their latest values. A shown row that left the choice set stays in
/// place and is listed in `unavailable`. New rows join at the bottom, up to
/// [limit] rows.
({List<MentionCandidate> rows, Set<String> unavailable}) stableMentionRows(
  List<MentionCandidate> previous,
  List<MentionCandidate> ranked, {
  int limit = mentionSuggestionLimit,
}) {
  final byKey = {for (final candidate in ranked) candidate.pubkey: candidate};
  final unavailable = <String>{};
  final rows = <MentionCandidate>[];
  final shown = <String>{};
  for (final row in previous) {
    if (!shown.add(row.pubkey)) continue;
    final latest = byKey[row.pubkey];
    if (latest == null) unavailable.add(row.pubkey);
    rows.add(latest ?? row);
  }
  for (final candidate in ranked) {
    if (rows.length >= limit) break;
    if (shown.add(candidate.pubkey)) rows.add(candidate);
  }
  return (rows: rows, unavailable: unavailable);
}
