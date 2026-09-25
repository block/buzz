import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../mentions/agent_identity_provider.dart';
import '../profile/user_cache_provider.dart';
import '../relay/relay.dart';
import 'identity_names.dart';

/// Client-wide naming facts. Views choose their own comparison context with
/// [IdentityNameSources.scope]; never resolve against this whole cache.
final identityNameSourcesProvider = Provider<IdentityNameSources>((ref) {
  return IdentityNameSources(
    profiles: ref.watch(userCacheProvider),
    agentPubkeys: ref.watch(knownAgentPubkeysProvider),
    agentDisplayNames: ref.watch(agentDirectoryDisplayNamesProvider),
    agentOwners: ref.watch(agentOwnersProvider).asData?.value ?? const {},
    viewer: ref.watch(myPubkeyProvider)?.toLowerCase(),
  );
});

/// Requests missing owner profiles for [names] after the current build.
void loadIdentityNameOwners(
  Ref ref,
  IdentityNameSources sources,
  Iterable<String> candidates,
) {
  final missing = sources.missingOwnerProfiles(candidates);
  if (missing.isEmpty) return;
  Future.microtask(() {
    if (ref.mounted) {
      ref.read(userCacheProvider.notifier).preload(missing.toList());
    }
  });
}

/// Labels for a displayed collection with no channel context, such as search
/// results, a Pulse timeline, or a picker's choices. [candidates] is that
/// collection; keys outside it resolve against it plus themselves.
IdentityNames watchIdentityNames(
  WidgetRef ref,
  Iterable<String> candidates, {
  Set<String> agentPubkeys = const {},
  Map<String, String> fallbackNames = const {},
}) {
  final sources = ref.watch(identityNameSourcesProvider);
  final names = sources.scope(
    candidates,
    agentPubkeys: agentPubkeys,
    fallbackNames: fallbackNames,
  );
  final missing = sources.missingOwnerProfiles(names.candidates);
  if (missing.isNotEmpty) {
    Future.microtask(() {
      if (ref.context.mounted) {
        ref.read(userCacheProvider.notifier).preload(missing.toList());
      }
    });
  }
  return names;
}
