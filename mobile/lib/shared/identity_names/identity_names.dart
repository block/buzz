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
  /// Owner: cached profile, then agent directory, then [fallbackOwner].
  NamingIdentity? factFor(
    String pubkey, {
    String? fallbackName,
    String? fallbackOwner,
    bool isAgent = false,
  }) {
    final key = pubkey.toLowerCase();
    if (!_hexPubkey.hasMatch(key)) return null;
    final profile = profiles[key];
    final owner = (profile?.ownerPubkey ?? agentOwners[key] ?? fallbackOwner)
        ?.toLowerCase();
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

  /// Owner lookup fact: only a real, non-blank owner profile name.
  NamingIdentity? _ownerFact(String owner) {
    final name = _nonBlank(profiles[owner]?.displayName);
    return name == null ? null : NamingIdentity(pubkey: owner, name: name);
  }

  /// Resolves labels for one view. [candidates] is the view's comparison
  /// context (for example channel members); [agentPubkeys] adds view-local
  /// agent roles such as channel bots; [fallbackNames] supplies view-local
  /// names used only when no profile or directory name exists;
  /// [ownerPubkeys] supplies view-local owner hints (for example from a
  /// search result that is not cached yet), used only when no cached profile
  /// or directory owner exists.
  IdentityNames scope(
    Iterable<String> candidates, {
    Set<String> agentPubkeys = const {},
    Map<String, String> fallbackNames = const {},
    Map<String, String> ownerPubkeys = const {},
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
    {
      for (final MapEntry(:key, :value) in ownerPubkeys.entries)
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
  final Map<String, String> _ownerPubkeys;
  final Map<String, ResolvedIdentityName?> _outside = {};
  late final Map<String, ResolvedIdentityName> _members = _resolve(_candidates);

  IdentityNames._(
    this._sources,
    this._candidates,
    this._agentPubkeys,
    this._fallbackNames,
    this._ownerPubkeys,
  );

  /// The comparison context's lowercase keys.
  Set<String> get candidates => _candidates;

  /// The same comparison context and view-local facts, resolved against
  /// [sources]. A destination opened from this view (a sheet or route) uses
  /// it with its live sources, so its labels follow later profile and owner
  /// changes instead of keeping the opener's snapshot.
  IdentityNames withSources(IdentityNameSources sources) =>
      identical(sources, _sources)
      ? this
      : IdentityNames._(
          sources,
          _candidates,
          _agentPubkeys,
          _fallbackNames,
          _ownerPubkeys,
        );

  /// Owner keys of the context (or of [keys] only) whose profile is not
  /// cached yet. Callers load them so readable owner prefixes can appear;
  /// none are invented meanwhile.
  Set<String> missingOwnerProfiles([Iterable<String>? keys]) =>
      (keys?.map((key) => key.toLowerCase()) ?? _candidates)
          .map(_factFor)
          .map((fact) => fact?.ownerPubkey)
          .nonNulls
          .where((owner) => !_sources.profiles.containsKey(owner))
          .toSet();

  NamingIdentity? _factFor(String key) => _sources.factFor(
    key,
    fallbackName: _fallbackNames[key],
    fallbackOwner: _ownerPubkeys[key],
    isAgent: _agentPubkeys.contains(key),
  );

  /// The resolved label and qualifier for [pubkey], or null for an invalid key.
  ResolvedIdentityName? resolve(String pubkey) {
    final key = pubkey.toLowerCase();
    if (_candidates.contains(key)) return _members[key];
    return _outside.putIfAbsent(key, () {
      if (!_hexPubkey.hasMatch(key)) return null;
      return _resolve({..._candidates, key})[key];
    });
  }

  /// The unambiguous display label for [pubkey], including any qualifier
  /// needed to distinguish identities with the same name and owner.
  /// A malformed key keeps its plain known name and is never disambiguated.
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
      final fact = _factFor(key);
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
