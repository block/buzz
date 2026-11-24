import 'dart:async';

import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/channels/saved_message_store.dart';
import '../../shared/profile/user_cache_provider.dart';
import '../../shared/profile/verified_profile_owner.dart';
import '../../shared/relay/relay.dart';
import '../../shared/relay/relay_author_provider.dart';
import 'timeline_message.dart';

/// The signed evidence [formatTimeline] judges cross-signer edits with:
/// verified owners by author, and the relay's trusted signing key.
typedef EditAuthority = ({Map<String, String> owners, String? relayKey});

/// Watches the evidence the cross-signer edits in [events] need, and asks
/// the user cache for the profiles that supply it. Call from a hook build.
///
/// The profiles are asked for on each connect while the edits show, so a
/// request that failed is made again. The cache skips authors it already
/// has a current answer for, with or without an attestation.
///
/// Nothing is watched for a timeline without such edits, so the relay is not
/// asked for its key and unrelated profile changes do not reformat it.
EditAuthority useEditAuthority(WidgetRef ref, List<NostrEvent> events) {
  final needs = useMemoized(() => editAuthorityNeeds(events), [events]);
  final authors = needs.owners.join(',');
  final connected =
      authors.isNotEmpty &&
      ref.watch(
        relaySessionProvider.select(
          (session) => session.status == SessionStatus.connected,
        ),
      );
  useEffect(() {
    if (connected) {
      ref.read(userCacheProvider.notifier).requestLinked(needs.owners);
    }
    return null;
  }, [authors, connected]);
  return (
    owners: authors.isEmpty
        ? const <String, String>{}
        : ref.watch(_editOwnersProvider(authors)),
    relayKey: needs.relayKey ? ref.watch(editRelayKeyProvider) : null,
  );
}

/// [useEditAuthority] for a notifier that formats once: reads the evidence
/// held now, requests missing profiles without waiting for them, and starts
/// no relay key lookup.
EditAuthority readEditAuthority(Ref ref, List<NostrEvent> events) {
  final needs = editAuthorityNeeds(events);
  if (needs.owners.isNotEmpty) {
    unawaited(ref.read(userCacheProvider.notifier).preload(needs.owners));
  }
  final profiles = ref.read(userCacheProvider);
  return (
    owners: {
      for (final author in needs.owners)
        if (profiles[author]?.ownerEvidence case final evidence?)
          author: ?verifiedEvidenceOwner(evidence, author),
    },
    relayKey: needs.relayKey
        ? ref.read(_learnedRelayKeyProvider).value ??
              ref.read(savedRelayAuthorProvider).value
        : null,
  );
}

/// Checks ownership for the Edit and Delete actions.
/// Returns true if [viewer] is [author], or if [author]'s signed profile
/// verifies [viewer] as its owner.
/// If a restored profile names [viewer] as owner and lacks a signed copy,
/// requests the profile again and returns false.
/// Call when actions open so the check reads the latest cached profile.
bool readCanManageMessage(WidgetRef ref, String author, String? viewer) {
  final key = author.toLowerCase();
  final me = viewer?.toLowerCase();
  if (me == null) return false;
  if (key == me) return true;
  final profile = ref.read(userCacheProvider)[key];
  final evidence = profile?.ownerEvidence;
  if (evidence == null) {
    // A restored profile without its signed copy names this viewer: fetch
    // the profile again so a later menu can verify the owner.
    if (profile?.ownerPubkey == me) {
      ref.read(userCacheProvider.notifier).get(key);
    }
    return false;
  }
  return verifiedEvidenceOwner(evidence, key) == me;
}

/// Verified owners for comma-joined [authors]. Rebuilt only when one of
/// their verified owners changes, so a timeline reformats only then.
final _editOwnersProvider = Provider.autoDispose
    .family<Map<String, String>, String>(
      (ref, authors) => {
        for (final author in authors.split(','))
          author: ?ref.watch(verifiedProfileOwnerProvider(author)),
      },
    );

/// The relay key last read live for this identity and relay.
class _LearnedRelayKey {
  String? value;
}

/// One per identity and relay, so a key never outlives its community.
final _learnedRelayKeyProvider = Provider<_LearnedRelayKey>((ref) {
  ref.watch(
    relayConfigProvider.select((config) => (config.baseUrl, config.nsec)),
  );
  return _LearnedRelayKey();
});

/// The relay signing key trusted for edit attribution.
///
/// While connected, asks the relay (NIP-11); a key it returns replaces any
/// earlier one at once, as the saved-message store does on rotation. While
/// offline, reconnecting or reloading, keeps the key last read for this
/// identity and relay, else the key the saved-message store holds. Watch it
/// only when an edit needs it: watching while connected costs a request.
final editRelayKeyProvider = Provider.autoDispose<String?>((ref) {
  final learned = ref.watch(_learnedRelayKeyProvider);
  final saved = ref.watch(savedRelayAuthorProvider).value;
  final connected = ref.watch(
    relaySessionProvider.select(
      (session) => session.status == SessionStatus.connected,
    ),
  );
  if (connected) {
    final relay = ref.watch(
      relayConfigProvider.select((config) => config.baseUrl),
    );
    final live = ref.watch(relayAuthorProvider(relay)).value;
    if (live != null) learned.value = live;
  }
  return learned.value ?? saved;
});
