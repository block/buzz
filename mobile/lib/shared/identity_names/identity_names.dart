import 'package:flutter/foundation.dart';

import '../profile/user_profile.dart';
import '../utils/string_utils.dart';
import 'identity_name_policy.dart';

final RegExp _hexPubkey = RegExp(r'^[0-9a-f]{64}$');

/// Immutable naming facts known to this client, independent of any view.
///
/// Mobile's owner hint is the verified NIP-OA owner from the profile `auth`
/// tag (or agent directory). It is display ranking only, never authority.
@immutable
class IdentityNameSources {
  final Map<String, UserProfile> profiles;
  final Set<String> agentPubkeys;
  final Map<String, String> agentDisplayNames;
  final Map<String, String> agentOwners;
  final String? viewer;

  const IdentityNameSources({
    this.profiles = const {},
    this.agentPubkeys = const {},
    this.agentDisplayNames = const {},
    this.agentOwners = const {},
    this.viewer,
  });

  static const empty = IdentityNameSources();

  /// The naming fact for [pubkey], or null when it is not a valid key.
  ///
  /// Name: non-blank profile display name, then agent-directory name, then
  /// [fallbackName], then the compact npub. The resolver trims the name.
  NamingIdentity? factFor(
    String pubkey, {
    String? fallbackName,
    bool isAgent = false,
  }) {
    final key = pubkey.toLowerCase();
    if (!_hexPubkey.hasMatch(key)) return null;
    final profile = profiles[key];
    final owner = (profile?.ownerPubkey ?? agentOwners[key])?.toLowerCase();
    return NamingIdentity(
      pubkey: key,
      name:
          _nonBlank(profile?.displayName) ??
          _nonBlank(agentDisplayNames[key]) ??
          _nonBlank(fallbackName) ??
          shortPubkey(key),
      isAgent: isAgent || owner != null || agentPubkeys.contains(key),
      ownerPubkey: owner != null && _hexPubkey.hasMatch(owner) ? owner : null,
    );
  }

  /// Owner keys of [candidates] whose profile is not cached yet. Callers load
  /// them so readable owner prefixes can appear; none are invented meanwhile.
  Set<String> missingOwnerProfiles(Iterable<String> candidates) => candidates
      .map((key) => factFor(key)?.ownerPubkey)
      .nonNulls
      .where((owner) => !profiles.containsKey(owner))
      .toSet();

  /// Owner lookup fact: only a real, non-blank owner profile name.
  NamingIdentity? _ownerFact(String owner) {
    final name = _nonBlank(profiles[owner]?.displayName);
    return name == null ? null : NamingIdentity(pubkey: owner, name: name);
  }

  /// Resolves labels for one view. [candidates] is the view's comparison
  /// context (for example channel members); [agentPubkeys] adds view-local
  /// agent roles such as channel bots; [fallbackNames] supplies view-local
  /// names used only when no profile or directory name exists.
  IdentityNames scope(
    Iterable<String> candidates, {
    Set<String> agentPubkeys = const {},
    Map<String, String> fallbackNames = const {},
  }) => IdentityNames._(
    this,
    {
      for (final key in candidates)
        if (_hexPubkey.hasMatch(key.toLowerCase())) key.toLowerCase(),
    },
    {for (final key in agentPubkeys) key.toLowerCase()},
    {
      for (final MapEntry(:key, :value) in fallbackNames.entries)
        key.toLowerCase(): value,
    },
  );
}

/// Contextual display labels for one view's comparison context.
///
/// Labels are presentation only: keep the exact key for navigation, mentions,
/// and every identity-bound action. A key outside the context is resolved
/// against the context plus that key, as the contract requires for
/// historical or non-member references.
class IdentityNames {
  final IdentityNameSources _sources;
  final Set<String> _candidates;
  final Set<String> _agentPubkeys;
  final Map<String, String> _fallbackNames;
  final Map<String, ResolvedIdentityName?> _outside = {};
  late final Map<String, ResolvedIdentityName> _members = _resolve(_candidates);

  IdentityNames._(
    this._sources,
    this._candidates,
    this._agentPubkeys,
    this._fallbackNames,
  );

  /// The comparison context's lowercase keys.
  Set<String> get candidates => _candidates;

  /// The resolved label and qualifier for [pubkey], or null for an invalid key.
  ResolvedIdentityName? resolve(String pubkey) {
    final key = pubkey.toLowerCase();
    if (_candidates.contains(key)) return _members[key];
    return _outside.putIfAbsent(key, () {
      if (!_hexPubkey.hasMatch(key)) return null;
      return _resolve({..._candidates, key})[key];
    });
  }

  /// The display label for [pubkey]. A malformed key is outside the naming
  /// contract: it keeps its plain known name and is never disambiguated.
  String labelFor(String pubkey) =>
      resolve(pubkey)?.name ?? _plainName(pubkey.toLowerCase());

  String _plainName(String key) =>
      _nonBlank(_sources.profiles[key]?.displayName)?.trim() ??
      _nonBlank(_sources.agentDisplayNames[key])?.trim() ??
      _nonBlank(_fallbackNames[key])?.trim() ??
      shortPubkey(key);

  Map<String, ResolvedIdentityName> _resolve(Set<String> candidates) {
    final facts = <NamingIdentity>[];
    final owners = <String>{};
    for (final key in candidates) {
      final fact = _sources.factFor(
        key,
        fallbackName: _fallbackNames[key],
        isAgent: _agentPubkeys.contains(key),
      );
      if (fact == null) continue;
      facts.add(fact);
      if (fact.ownerPubkey case final owner? when !candidates.contains(owner)) {
        owners.add(owner);
      }
    }
    // Owners outside the context are lookup facts, not collision candidates;
    // placing them first keeps a candidate's own fact preferred.
    final ownerFacts = [
      for (final owner in owners) ?_sources._ownerFact(owner),
    ];
    return resolveIdentityNames(
      [...ownerFacts, ...facts],
      viewer: _sources.viewer,
      candidates: candidates,
    );
  }
}

String? _nonBlank(String? value) {
  if (value == null) return null;
  final trimmed = trimIdentityName(value);
  return trimmed.isEmpty ? null : value;
}
