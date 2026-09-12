import 'dart:convert';
import 'dart:typed_data';

import 'package:nostr/nostr.dart' as nostr;
import 'package:pointycastle/digests/sha256.dart';

import '../relay/nostr_models.dart';
import 'signed_event.dart';

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
/// Returns the owner only for one valid auth tag on a signed kind:0 envelope.
String? verifiedOaOwnerPubkey(NostrEvent event) {
  final tags = event.tags.where((tag) => tag.isNotEmpty && tag[0] == 'auth');
  if (event.kind != 0 || tags.length != 1 || !verifySignedEvent(event)) {
    return null;
  }
  final agent = event.pubkey;

  for (final tag in tags) {
    if (tag.length != 4 || tag[0] != 'auth') continue;

    final owner = tag[1];
    final conditions = tag[2];
    final sig = tag[3];

    // Self-attestation is meaningless and rejected.
    if (owner == agent) continue;
    if (!RegExp(r'^[0-9a-f]{64}$').hasMatch(owner) ||
        !RegExp(r'^[0-9a-f]{128}$').hasMatch(sig)) {
      continue;
    }
    if (!_validConditions(conditions, event)) continue;

    final preimage = utf8.encode('nostr:agent-auth:$agent:$conditions');
    final digest = SHA256Digest().process(Uint8List.fromList(preimage));
    final message = digest
        .map((b) => b.toRadixString(16).padLeft(2, '0'))
        .join();

    try {
      if (nostr.Schnorr.verify(
        publicKey: owner,
        message: message,
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

/// Select the latest authenticated profile, including signed revocations.
/// NIP-01 ties choose the lowest event id, independent of response order.
Map<String, NostrEvent> latestProfileEvents(Iterable<NostrEvent> events) {
  final latest = <String, NostrEvent>{};
  for (final event in events.where((event) => event.kind == 0)) {
    if (!verifySignedEvent(event)) continue;
    final key = event.pubkey.toLowerCase();
    final previous = latest[key];
    if (previous == null ||
        event.createdAt > previous.createdAt ||
        (event.createdAt == previous.createdAt &&
            event.id.compareTo(previous.id) < 0)) {
      latest[key] = event;
    }
  }
  return latest;
}

/// Validate the NIP-OA `conditions` string: empty, or `&`-joined clauses of
/// `kind=<n>`, `created_at<<n>`, or `created_at><n>` with canonical decimals.
bool _validConditions(String conditions, NostrEvent event) {
  if (conditions.isEmpty) return true;
  if (conditions.contains(RegExp(r'\s'))) return false;

  for (final clause in conditions.split('&')) {
    final match = RegExp(
      r'^(?:kind=|created_at<|created_at>)(0|[1-9][0-9]*)$',
    ).firstMatch(clause);
    if (match == null) return false;
    final value = int.tryParse(match.group(1)!);
    if (value == null || value > 4294967295) return false;
    if (clause.startsWith('kind=') && (value > 65535 || value != event.kind)) {
      return false;
    }
    if (clause.startsWith('created_at<') && event.createdAt >= value) {
      return false;
    }
    if (clause.startsWith('created_at>') && event.createdAt <= value) {
      return false;
    }
  }

  return true;
}
