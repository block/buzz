import 'package:flutter/foundation.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../crypto/nip_oa.dart';
import '../relay/nostr_models.dart';
import 'user_cache_provider.dart';

/// Verification results by signed profile. The user cache keeps one event
/// object per winning profile, so a repaint or a reopened thread reuses the
/// result, and a newer profile is a new object that is checked again.
final _verifiedOwners = Expando<({String? owner})>();

/// How many signed profiles [verifiedEvidenceOwner] has checked, for tests
/// that bound when the check runs.
@visibleForTesting
int verifiedOwnerChecks = 0;

/// The owner that [evidence] attests for [pubkey]: the profile must be
/// [pubkey]'s own, with a valid id and signature and a valid owner
/// attestation (see [verifiedProfileOaOwnerPubkey]). Null otherwise.
String? verifiedEvidenceOwner(NostrEvent evidence, String pubkey) {
  if (evidence.pubkey.toLowerCase() != pubkey.toLowerCase()) return null;
  return (_verifiedOwners[evidence] ??= () {
    verifiedOwnerChecks++;
    return (owner: verifiedProfileOaOwnerPubkey(evidence));
  }()).owner;
}

/// The verified owner of [pubkey], from the signed profile the user cache
/// holds for it. Null when it has no attestation, or its signed profile has
/// not been fetched yet (such as an entry restored from an older snapshot).
///
/// A newer profile replaces the evidence, so one that drops the attestation
/// clears the owner at once. Profiles are not requested here; callers ask
/// the user cache for the identities they show.
final verifiedProfileOwnerProvider = Provider.autoDispose
    .family<String?, String>((ref, pubkey) {
      final key = pubkey.toLowerCase();
      final evidence = ref.watch(
        userCacheProvider.select((cache) => cache[key]?.ownerEvidence),
      );
      return evidence == null ? null : verifiedEvidenceOwner(evidence, key);
    });
