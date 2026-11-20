part of '../compose_bar.dart';

/// How often chip labels walked the roster and ran the naming policy, for
/// tests: neither may follow a keystroke, and a profile that is nobody's
/// namesake costs no naming pass.
@visibleForTesting
int mentionQualifierRosterPasses = 0;

/// See [mentionQualifierRosterPasses].
@visibleForTesting
int mentionQualifierNamingPasses = 0;

/// The roster by lowercase key with each member's position, the first entry
/// winning as in [buildMentionCandidates].
typedef _QualifierRoster = Map<String, (int, ChannelMember)>;

_QualifierRoster _qualifierRoster(List<ChannelMember> members) {
  final roster = <String, (int, ChannelMember)>{};
  for (final (index, member) in members.indexed) {
    roster.putIfAbsent(member.pubkey.toLowerCase(), () => (index, member));
  }
  return roster;
}

/// The naming fact [mentionPickerNames] derives for one key from the
/// [candidates] that carry it: the last non-blank name and owner win and any
/// agent mark counts.
NamingIdentity? _pickerFact(
  IdentityNameSources sources,
  String key,
  Iterable<MentionCandidate> candidates,
) {
  String? name;
  String? owner;
  var isAgent = false;
  for (final candidate in candidates) {
    if (candidate.displayName?.trim().isNotEmpty == true) {
      name = candidate.displayName;
    }
    owner = candidate.ownerPubkey ?? owner;
    isAgent = isAgent || candidate.isAgent;
  }
  return sources.factFor(
    key,
    fallbackName: name,
    fallbackOwner: owner,
    isAgent: isAgent,
  );
}

/// The part of the channel that decides the chips' labels.
///
/// The naming policy only ever changes the label of identities that share a
/// label, so the chips are named exactly as among the whole roster when
/// they are named among the identities whose labels can meet theirs. A
/// label is the name, an agent's `Owner’s Name` or `Name (agent)`, or one
/// of those with a key suffix after ` · `. [candidates] is therefore the
/// chips, every member with one of those labels in common with an identity
/// already taken, every member whose label is another's up to a ` · `, and
/// the owners among the members of the agents taken, in roster order with
/// the chips last. The walk reads each member's name and builds nothing
/// else for the rest of the roster.
///
/// [signature] is everything the labels are computed from. While it is
/// equal, the labels are too, so a profile, verified name, owner or roster
/// change that touches none of these identities needs no naming pass.
({List<MentionCandidate> candidates, List<Object?> signature})
_chipNamingContext(
  IdentityNameSources sources,
  _QualifierRoster roster,
  Map<String, UserProfile> userCache,
  Map<String, String> agentOwners,
  List<MentionCandidate> chips,
) {
  const join = ' · ';
  final picked = <String, List<MentionCandidate>>{};
  for (final chip in chips) {
    (picked[chip.pubkey.toLowerCase()] ??= []).add(chip);
  }
  MentionCandidate memberCandidate(ChannelMember member) =>
      memberMentionCandidate(
        member,
        userCache: userCache,
        ownerByAgentPubkey: agentOwners,
      );
  NamingIdentity? factOf(String key) {
    final member = roster[key]?.$2;
    final chosen = picked[key];
    if (member == null && chosen == null) return null;
    return _pickerFact(sources, key, [
      if (member != null) memberCandidate(member),
      ...?chosen,
    ]);
  }

  // Every label the policy can give an identity before a key suffix. An
  // owner outside the channel and the chips is named by its verified or
  // profile name; both are tried.
  List<String> labelsOf(NamingIdentity fact) {
    final name = trimIdentityName(fact.name);
    if (!fact.isAgent) return [name];
    final owner = fact.ownerPubkey;
    final ownerFact = owner == null ? null : factOf(owner);
    return [
      name,
      '$name (agent)',
      if (owner != null)
        for (final ownerName
            in ownerFact != null
                ? [ownerFact.name]
                : [
                    sources.verifiedNames[owner],
                    sources.profiles[owner]?.displayName,
                  ])
          if (ownerName != null && trimIdentityName(ownerName).isNotEmpty)
            '${trimIdentityName(ownerName)}’s $name',
    ];
  }

  final taken = <String, NamingIdentity>{};
  final labels = <String>{};
  final stems = <String>{};
  final outsideOwners = <String>{};
  void take(String key, NamingIdentity fact) {
    if (taken.containsKey(key)) return;
    taken[key] = fact;
    for (final label in labelsOf(fact)) {
      labels.add(label);
      for (
        var i = label.indexOf(join);
        i >= 0;
        i = label.indexOf(join, i + 1)
      ) {
        stems.add(label.substring(0, i));
      }
    }
    final owner = fact.ownerPubkey;
    if (owner == null) return;
    final ownerFact = factOf(owner);
    if (ownerFact != null) {
      take(owner, ownerFact);
    } else {
      outsideOwners.add(owner);
    }
  }

  bool meets(String label) {
    if (labels.contains(label)) return true;
    if (stems.isNotEmpty && stems.contains(label)) return true;
    for (var i = label.indexOf(join); i >= 0; i = label.indexOf(join, i + 1)) {
      if (labels.contains(label.substring(0, i))) return true;
    }
    return false;
  }

  // Most members are people whose one label meets nothing. Their name is
  // read here as [IdentityNameSources.factFor] chooses it, with nothing
  // built; an agent, or a member named only by key, takes the full path.
  final sameCache = identical(userCache, sources.profiles);
  final owners = [
    for (final owners in {agentOwners, sources.agentOwners}.toList())
      if (owners.isNotEmpty) owners,
  ];
  final verified = sources.verifiedNames.isEmpty ? null : sources.verifiedNames;
  final directory = sources.agentDisplayNames.isEmpty
      ? null
      : sources.agentDisplayNames;
  String? named(String? name) {
    if (name == null) return null;
    final trimmed = trimIdentityName(name);
    return trimmed.isEmpty ? null : trimmed;
  }

  String? plainName(String key, ChannelMember member) {
    final profile = sources.profiles[key];
    if (member.isBot ||
        profile?.ownerPubkey != null ||
        (!sameCache && userCache[key]?.ownerPubkey != null) ||
        owners.any((owners) => owners.containsKey(key)) ||
        sources.agentPubkeys.contains(key)) {
      return null;
    }
    final fallback = member.displayName;
    return named(verified?[key]) ??
        named(profile?.displayName) ??
        named(directory?[key]) ??
        (fallback == null || fallback.trim().isEmpty
            ? null
            : trimIdentityName(fallback));
  }

  for (final key in picked.keys) {
    final fact = factOf(key);
    if (fact != null) take(key, fact);
  }
  // Again while a pass found a label the members before it were not
  // compared with.
  var compared = -1;
  while (labels.length + stems.length != compared) {
    compared = labels.length + stems.length;
    mentionQualifierRosterPasses++;
    for (final MapEntry(:key, value: (_, member)) in roster.entries) {
      if (taken.containsKey(key)) continue;
      if (plainName(key, member) case final name? when !meets(name)) continue;
      final fact = _pickerFact(sources, key, [memberCandidate(member)]);
      if (fact == null) continue;
      if (fact.isAgent
          ? labelsOf(fact).any(meets)
          : meets(trimIdentityName(fact.name))) {
        take(key, fact);
      }
    }
  }

  final members = [for (final key in taken.keys) ?roster[key]]
    ..sort((a, b) => a.$1.compareTo(b.$1));
  return (
    candidates: [
      for (final (_, member) in members) memberCandidate(member),
      ...chips,
    ],
    signature: [
      sources.viewer,
      for (final chip in chips) ...[chip.pubkey.toLowerCase(), chip.label],
      ...taken.values,
      for (final owner in outsideOwners) ...[
        owner,
        sources.verifiedNames[owner],
        sources.profiles[owner]?.displayName,
      ],
    ],
  );
}

/// Display labels for chips whose name another candidate shares, by
/// lowercase key: the picked name with the chooser's short key suffix
/// (`Sam · a1b2`), from the same identity-name policy. [context] is the
/// comparison context from [_chipNamingContext], which names the chips as
/// the channel's members and the chips together would, so a single picked
/// namesake is qualified, and so is a person picked from the directory who
/// shares a member's name. The key never enters the text.
Map<String, String> _namedChipQualifiers(
  IdentityNameSources sources,
  List<MentionCandidate> context,
  List<MentionCandidate> chips,
) {
  mentionQualifierNamingPasses++;
  final names = mentionPickerNames(sources, context);
  return {
    for (final chip in chips)
      if (names.resolve(chip.pubkey)?.qualifier case final suffix?)
        chip.pubkey.toLowerCase(): '${chip.label} · $suffix',
  };
}

/// The chip labels for [chips] among [members], as the composer computes
/// them. Empty without a chip, before any member is read.
@visibleForTesting
Map<String, String> mentionChipQualifiers({
  required IdentityNameSources sources,
  required List<ChannelMember> members,
  required Map<String, UserProfile> userCache,
  required Map<String, String> agentOwners,
  required List<MentionCandidate> chips,
}) {
  if (chips.isEmpty) return const {};
  final context = _chipNamingContext(
    sources,
    _qualifierRoster(members),
    userCache,
    agentOwners,
    chips,
  );
  return _namedChipQualifiers(sources, context.candidates, chips);
}

/// What [_useMentionChipQualifiers] last computed: the inputs it walked the
/// roster for, the naming context's signature and the labels.
typedef _NamedChips = ({
  List<Object?> inputs,
  String chips,
  List<Object?> signature,
  Map<String, String> qualifiers,
});

/// Keeps the editor's namesake chip labels current: a chip whose name
/// another candidate shares reads `Sam · a1b2`, the chooser's label, while
/// the draft text stays `@Sam`.
///
/// A draft without a chip reads no member. With a chip, a change to the
/// chips, the naming facts or [roster] walks the roster once for the chips'
/// namesakes ([_chipNamingContext]) and names only those; when the change
/// touched none of them the labels stand and nothing is named. [roster] is
/// indexed by key once per list. An edit that leaves the chips as they
/// were, or takes them away and puts them back, does none of this.
///
/// The chips are read from the controller as it changes rather than from a
/// rebuild: a prefill that swaps one namesake agent for another changes
/// them from inside a build, where a rebuild request is dropped.
void _useMentionChipQualifiers({
  required WidgetRef ref,
  required _MarkdownEditingController controller,
  required List<ChannelMember> roster,
  required Map<String, UserProfile> userCache,
  required Map<String, String>? agentOwners,
}) {
  final sources = ref.watch(identityNameSourcesProvider);
  final indexed = useRef<(List<ChannelMember>, _QualifierRoster)?>(null);
  final named = useRef<_NamedChips?>(null);
  final current = useRef<List<Object?>?>(null);
  useEffect(() {
    final inputs = current.value = [sources, roster, userCache, agentOwners];
    void update() {
      // The effect these inputs replaced still listens while this one
      // starts, and would label the chips from the facts it was given.
      if (!identical(current.value, inputs)) return;
      final mentions = controller.mentions;
      if (mentions.isEmpty) {
        controller.setMentionQualifiers(const {});
        return;
      }
      final chips = [for (final mention in mentions) mention.candidate];
      final key = [
        for (final chip in chips) '${chip.pubkey.toLowerCase()} ${chip.label}',
      ].join('\n');
      final last = named.value;
      if (last != null &&
          last.chips == key &&
          Iterable<int>.generate(
            inputs.length,
          ).every((i) => identical(last.inputs[i], inputs[i]))) {
        controller.setMentionQualifiers(last.qualifiers);
        return;
      }
      if (!identical(indexed.value?.$1, roster)) {
        indexed.value = (roster, _qualifierRoster(roster));
      }
      final context = _chipNamingContext(
        sources,
        indexed.value!.$2,
        userCache,
        agentOwners ?? const {},
        chips,
      );
      final qualifiers =
          last != null && listEquals(last.signature, context.signature)
          ? last.qualifiers
          : _namedChipQualifiers(sources, context.candidates, chips);
      named.value = (
        inputs: inputs,
        chips: key,
        signature: context.signature,
        qualifiers: qualifiers,
      );
      controller.setMentionQualifiers(qualifiers);
    }

    update();
    controller.addListener(update);
    return () => controller.removeListener(update);
  }, [controller, sources, roster, userCache, agentOwners]);
}
