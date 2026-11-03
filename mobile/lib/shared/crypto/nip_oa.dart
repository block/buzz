import 'dart:convert';
import 'dart:typed_data';

import 'package:buzz/shared/crypto/event_signing.dart';
import 'package:pointycastle/digests/sha256.dart';
import '../relay/nostr_models.dart';

/// Verify the signed profile and all owner-attestation conditions against it.
String? verifiedProfileOaOwnerPubkey(NostrEvent event) {
  if (event.kind != 0) return null;
  final tags = event.tags
      .where((tag) => tag.isNotEmpty && tag[0] == 'auth')
      .toList();
  if (tags.length != 1 || tags.single.length != 4) return null;
  try {
    verifiedNostrEventFromJson(jsonEncode(event.toJson()));
  } catch (_) {
    return null;
  }
  final owner = verifiedOaOwnerPubkey(event.tags, event.pubkey);
  if (owner == null) return null;
  final conditions = tags.single[2];
  for (final clause
      in conditions.isEmpty ? <String>[] : conditions.split('&')) {
    if (clause.startsWith('kind=')) {
      if (int.parse(clause.substring(5)) != event.kind) return null;
    } else if (clause.startsWith('created_at<')) {
      if (event.createdAt >= int.parse(clause.substring(11))) return null;
    } else if (clause.startsWith('created_at>')) {
      if (event.createdAt <= int.parse(clause.substring(11))) return null;
    }
  }
  return owner;
}

/// NIP-OA (Owner Attestation) — verify the `auth` tag on a kind:0 profile
/// that proves an owner key authorized an agent key.
///
/// Tag format: ["auth", "<owner-pubkey-hex>", "<conditions>", "<sig-hex>"]
/// Preimage:   "nostr:agent-auth:" + agent_pubkey_hex + ":" + conditions
/// Signature:  BIP-340 Schnorr over SHA256(preimage) by the owner key.
///
/// Mirrors `profile_valid_oa_owner_pubkey` in desktop/src-tauri: the tag is
/// verified against the profile event author, so a forged or stale marker
/// cannot turn a person into an agent.
///
/// Low-level tag verification. Use verifiedProfileOaOwnerPubkey for admission.
/// Returns the owner pubkey (lowercase hex) for exactly one valid auth tag,
/// or null if none verifies.
String? verifiedOaOwnerPubkey(List<List<String>> tags, String agentPubkey) {
  final agent = agentPubkey.toLowerCase();
  if (tags.where((tag) => tag.isNotEmpty && tag[0] == 'auth').length != 1) {
    return null;
  }

  for (final tag in tags) {
    if (tag.length != 4 || tag[0] != 'auth') continue;

    final owner = tag[1];
    if (!RegExp(r'^[0-9a-f]{64}$').hasMatch(owner) ||
        !RegExp(r'^[0-9a-f]{128}$').hasMatch(tag[3])) {
      continue;
    }
    final conditions = tag[2];
    final sig = tag[3];

    // Self-attestation is meaningless and rejected.
    if (owner == agent) continue;
    if (owner.length != 64 || sig.length != 128) continue;
    if (!_validConditions(conditions)) continue;

    final preimage = utf8.encode('nostr:agent-auth:$agent:$conditions');
    final digest = SHA256Digest().process(Uint8List.fromList(preimage));
    final message = digest
        .map((b) => b.toRadixString(16).padLeft(2, '0'))
        .join();

    try {
      if (verifySchnorrHex(
        publicKey: owner,
        messageHex: message,
        signature: sig,
      )) {
        return owner;
      }
    } catch (_) {
      // Malformed hex — treat as an invalid tag.
    }
  }

  return null;
}

/// Validate the NIP-OA `conditions` string: empty, or `&`-joined clauses of
/// `kind=<n>`, `created_at<<n>`, or `created_at><n>` with canonical decimals.
bool _validConditions(String conditions) {
  if (conditions.isEmpty) return true;
  if (conditions.contains(RegExp(r'\s'))) return false;

  for (final clause in conditions.split('&')) {
    final match = RegExp(
      r'^(?:kind=|created_at<|created_at>)(0|[1-9][0-9]*)$',
    ).firstMatch(clause);
    if (match == null) return false;
    final value = int.tryParse(match.group(1)!);
    if (value == null || value > 4294967295) return false;
    if (clause.startsWith('kind=') && value > 65535) return false;
  }

  return true;
}
