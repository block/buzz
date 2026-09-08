part of '../compose_bar.dart';

Set<String> _agentMentionLabels({
  required Iterable<MentionCandidate> candidates,
}) {
  return <String>{
    for (final candidate in candidates)
      if (candidate.isAgent) candidate.label,
  };
}
