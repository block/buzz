import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../identity_names/identity_names.dart';
import '../identity_names/identity_names_provider.dart';
import '../mentions/agent_identity_provider.dart';
import '../profile/user_profile.dart';
import 'me_assistants.dart';

/// [base] with the owner's inventory name ahead of each agent's self-published
/// profile name, as Solo has always shown it. Verified names still win, and
/// every other naming fact is untouched.
IdentityNameSources meAssistantNameSources(
  IdentityNameSources base,
  List<AgentDirectoryEntry> agents,
) {
  final inventory = {
    for (final agent in agents)
      if (agent.displayName?.trim().isNotEmpty == true)
        agent.pubkey.toLowerCase(): agent.displayName!,
  };
  return IdentityNameSources(
    // An uncached profile stays absent so the caller still loads it.
    profiles: {
      for (final MapEntry(:key, :value) in base.profiles.entries)
        key: inventory.containsKey(key)
            ? UserProfile(
                pubkey: value.pubkey,
                displayName: inventory[key],
                avatarUrl: value.avatarUrl,
                about: value.about,
                nip05Handle: value.nip05Handle,
                ownerPubkey: value.ownerPubkey,
              )
            : value,
    },
    verifiedNames: base.verifiedNames,
    agentPubkeys: base.agentPubkeys,
    agentDisplayNames: {...base.agentDisplayNames, ...inventory},
    agentOwners: base.agentOwners,
    viewer: base.viewer,
  );
}

/// Labels for the agents Solo offers. The comparison context is exactly
/// [agents] (the live inventory, archived agents already excluded), so a
/// hidden archived namesake never adds a qualifier, and the picker, draft and
/// header all read the same label for one agent.
IdentityNames meAssistantNames(
  IdentityNameSources sources,
  List<AgentDirectoryEntry> agents,
) {
  final keys = [for (final agent in agents) agent.pubkey];
  return meAssistantNameSources(
    sources,
    agents,
  ).scope(keys, agentPubkeys: keys.toSet());
}

/// [meAssistantNames] for the live inventory, built once for every reader so
/// each Solo row does not copy the profile cache again.
final meAssistantNamesProvider = Provider.autoDispose<IdentityNames>(
  (ref) => meAssistantNames(
    ref.watch(identityNameSourcesProvider),
    ref.watch(meAssistantsProvider).value ?? const [],
  ),
);

/// [meAssistantNames] for a widget, loading the profiles it still lacks.
/// [saved] are agents a saved choice still names that Solo no longer offers,
/// such as an archived agent: they are labeled as agents against [agents].
IdentityNames watchMeAssistantNames(
  WidgetRef ref,
  List<AgentDirectoryEntry> agents, {
  Iterable<String?> saved = const [],
}) {
  final keys = [for (final agent in agents) agent.pubkey];
  return watchIdentityNames(
    ref,
    keys,
    agentPubkeys: {...keys, ...saved.nonNulls},
    sources: meAssistantNameSources(
      ref.watch(identityNameSourcesProvider),
      agents,
    ),
  );
}
