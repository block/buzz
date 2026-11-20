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

/// The most changed keys [_ChipNaming.standsAfter] looks at one by one. A
/// larger change, such as a batch of profiles loading, walks the roster.
const _mostChangedKeys = 32;

/// Adds to [into] the keys whose value differs between [before] and
/// [after], and answers false once there are more than [_mostChangedKeys].
///
/// The published maps keep their order when an entry is replaced or
/// appended, so the two are read side by side and an unchanged entry costs
/// a pointer comparison; nothing is looked up or built for it. Maps whose
/// keys are out of step are compared by lookup.
bool _addChangedKeys<V>(
  Map<String, V> before,
  Map<String, V> after,
  Set<String> into,
) {
  if (identical(before, after)) return true;
  bool differs(V? a, V? b) => !identical(a, b) && a != b;
  final oldKeys = before.keys.iterator;
  final oldValues = before.values.iterator;
  final newKeys = after.keys.iterator;
  final newValues = after.values.iterator;
  var inStep = true;
  while (newKeys.moveNext()) {
    newValues.moveNext();
    final key = newKeys.current;
    if (!oldKeys.moveNext()) {
      // Appended entries.
      into.add(key);
      if (into.length > _mostChangedKeys) return false;
      continue;
    }
    oldValues.moveNext();
    if (!identical(key, oldKeys.current) && key != oldKeys.current) {
      inStep = false;
      break;
    }
    if (differs(oldValues.current, newValues.current)) {
      into.add(key);
      if (into.length > _mostChangedKeys) return false;
    }
  }
  if (inStep && !oldKeys.moveNext()) return true;
  for (final MapEntry(:key, :value) in after.entries) {
    if (differs(before[key], value) || !before.containsKey(key)) into.add(key);
    if (into.length > _mostChangedKeys) return false;
  }
  for (final key in before.keys) {
    if (!after.containsKey(key)) into.add(key);
    if (into.length > _mostChangedKeys) return false;
  }
  return true;
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
/// the chips last. [walk] reads each member's name and builds nothing else
/// for the rest of the roster.
///
/// [signature] is everything the labels are computed from. While it is
/// equal, the labels are too, so a profile, verified name, owner or roster
/// change that touches none of these identities needs no naming pass.
class _ChipNaming {
  _ChipNaming(
    this.sources,
    this.roster,
    this.userCache,
    this.agentOwners,
    this.chips,
  ) : _taken = {},
      _labels = {},
      _stems = {},
      _outsideOwners = {},
      _owners = {} {
    for (final chip in chips) {
      (_picked[chip.pubkey.toLowerCase()] ??= []).add(chip);
    }
  }

  /// [last]'s context under newer naming facts, for [standsAfter].
  _ChipNaming._after(
    _ChipNaming last,
    this.sources,
    this.userCache,
    this.agentOwners,
  ) : roster = last.roster,
      chips = last.chips,
      _taken = last._taken,
      _labels = last._labels,
      _stems = last._stems,
      _outsideOwners = last._outsideOwners,
      _owners = last._owners {
    _picked.addAll(last._picked);
  }

  static const _join = ' · ';

  final IdentityNameSources sources;
  final _QualifierRoster roster;
  final Map<String, UserProfile> userCache;
  final Map<String, String> agentOwners;
  final List<MentionCandidate> chips;

  final _picked = <String, List<MentionCandidate>>{};

  /// The identities in the context, with every label the policy can give
  /// them ([_labels]) and each label's part before a ` · ` ([_stems]).
  final Map<String, NamingIdentity> _taken;
  final Set<String> _labels;
  final Set<String> _stems;

  /// Owners of the agents taken that are neither members nor chips.
  final Set<String> _outsideOwners;

  /// The owner of every agent among the members and the chips, taken or
  /// not: a renamed owner changes its agents' labels.
  final Set<String> _owners;

  late final _sameCache = identical(userCache, sources.profiles);
  late final _ownerMaps = [
    for (final owners in {agentOwners, sources.agentOwners}.toList())
      if (owners.isNotEmpty) owners,
  ];
  late final _verified = sources.verifiedNames.isEmpty
      ? null
      : sources.verifiedNames;
  late final _directory = sources.agentDisplayNames.isEmpty
      ? null
      : sources.agentDisplayNames;

  MentionCandidate _memberCandidate(ChannelMember member) =>
      memberMentionCandidate(
        member,
        userCache: userCache,
        ownerByAgentPubkey: agentOwners,
      );

  NamingIdentity? _factOf(String key) {
    final member = roster[key]?.$2;
    final chosen = _picked[key];
    if (member == null && chosen == null) return null;
    return _pickerFact(sources, key, [
      if (member != null) _memberCandidate(member),
      ...?chosen,
    ]);
  }

  /// Every label the policy can give an identity before a key suffix. An
  /// owner outside the channel and the chips is named by its verified or
  /// profile name; both are tried.
  List<String> _labelsOf(NamingIdentity fact) {
    final name = trimIdentityName(fact.name);
    if (!fact.isAgent) return [name];
    final owner = fact.ownerPubkey;
    final ownerFact = owner == null ? null : _factOf(owner);
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

  void _take(String key, NamingIdentity fact) {
    if (_taken.containsKey(key)) return;
    _taken[key] = fact;
    for (final label in _labelsOf(fact)) {
      _labels.add(label);
      for (
        var i = label.indexOf(_join);
        i >= 0;
        i = label.indexOf(_join, i + 1)
      ) {
        _stems.add(label.substring(0, i));
      }
    }
    final owner = fact.ownerPubkey;
    if (owner == null) return;
    _owners.add(owner);
    final ownerFact = _factOf(owner);
    if (ownerFact != null) {
      _take(owner, ownerFact);
    } else {
      _outsideOwners.add(owner);
    }
  }

  bool _meets(String label) {
    if (_labels.contains(label)) return true;
    if (_stems.isNotEmpty && _stems.contains(label)) return true;
    for (
      var i = label.indexOf(_join);
      i >= 0;
      i = label.indexOf(_join, i + 1)
    ) {
      if (_labels.contains(label.substring(0, i))) return true;
    }
    return false;
  }

  static String? _named(String? name) {
    if (name == null) return null;
    final trimmed = trimIdentityName(name);
    return trimmed.isEmpty ? null : trimmed;
  }

  /// Most members are people whose one label meets nothing. Their name is
  /// read here as [IdentityNameSources.factFor] chooses it, with nothing
  /// built; an agent, or a member named only by key, takes the full path.
  String? _plainName(String key, ChannelMember member) {
    final profile = sources.profiles[key];
    if (member.isBot ||
        profile?.ownerPubkey != null ||
        (!_sameCache && userCache[key]?.ownerPubkey != null) ||
        _ownerMaps.any((owners) => owners.containsKey(key)) ||
        sources.agentPubkeys.contains(key)) {
      return null;
    }
    final fallback = member.displayName;
    return _named(_verified?[key]) ??
        _named(profile?.displayName) ??
        _named(_directory?[key]) ??
        (fallback == null || fallback.trim().isEmpty
            ? null
            : trimIdentityName(fallback));
  }

  /// The fact of a member outside the context whose label meets one in
  /// it, or null. Notes the owner of an agent it leaves out.
  NamingIdentity? _joining(String key, ChannelMember member) {
    if (_plainName(key, member) case final name? when !_meets(name)) {
      return null;
    }
    final fact = _pickerFact(sources, key, [_memberCandidate(member)]);
    if (fact == null) return null;
    if (fact.isAgent
        ? _labelsOf(fact).any(_meets)
        : _meets(trimIdentityName(fact.name))) {
      return fact;
    }
    if (fact.ownerPubkey case final owner?) _owners.add(owner);
    return null;
  }

  /// Finds the context among the whole roster.
  void walk() {
    for (final key in _picked.keys) {
      final fact = _factOf(key);
      if (fact != null) _take(key, fact);
    }
    // Again while a pass found a label the members before it were not
    // compared with.
    var compared = -1;
    while (_labels.length + _stems.length != compared) {
      compared = _labels.length + _stems.length;
      mentionQualifierRosterPasses++;
      for (final MapEntry(:key, value: (_, member)) in roster.entries) {
        if (_taken.containsKey(key)) continue;
        if (_joining(key, member) case final fact?) _take(key, fact);
      }
    }
  }

  /// [last]'s context under newer naming facts, when the change between
  /// the two cannot alter it; null when the roster has to be walked.
  ///
  /// The facts are compared entry by entry ([_addChangedKeys]) and only
  /// the keys whose naming inputs changed are looked at. The context
  /// stands unless one of them is a chip, an identity in the context (so a
  /// member who gives up a chip's name is found: its old label met), the
  /// owner of an agent among the members or the chips, or a member whose
  /// new label meets a label in the context. Everyone else is named from
  /// inputs that did not change, so a walk would take the same identities
  /// with the same facts and [signature] would be equal. No member whose
  /// inputs are unchanged is read.
  static _ChipNaming? standsAfter(
    _ChipNaming last,
    IdentityNameSources sources,
    Map<String, UserProfile> userCache,
    Map<String, String> agentOwners,
  ) {
    final from = last.sources;
    if (sources.viewer != from.viewer) return null;
    final changed = <String>{};
    if (!_addChangedKeys(from.profiles, sources.profiles, changed) ||
        !_addChangedKeys(last.userCache, userCache, changed) ||
        !_addChangedKeys(from.verifiedNames, sources.verifiedNames, changed) ||
        !_addChangedKeys(
          from.agentDisplayNames,
          sources.agentDisplayNames,
          changed,
        ) ||
        !_addChangedKeys(from.agentOwners, sources.agentOwners, changed) ||
        !_addChangedKeys(last.agentOwners, agentOwners, changed)) {
      return null;
    }
    final agents = sources.agentPubkeys;
    if (!identical(agents, from.agentPubkeys)) {
      for (final (now, then) in [
        (agents, from.agentPubkeys),
        (from.agentPubkeys, agents),
      ]) {
        for (final key in now) {
          if (!then.contains(key)) changed.add(key);
          if (changed.length > _mostChangedKeys) return null;
        }
      }
    }
    final next = _ChipNaming._after(last, sources, userCache, agentOwners);
    for (final key in changed) {
      if (next._taken.containsKey(key) || next._owners.contains(key)) {
        return null;
      }
      final member = next.roster[key]?.$2;
      if (member != null && next._joining(key, member) != null) return null;
    }
    return next;
  }

  List<MentionCandidate> get candidates {
    final members = [for (final key in _taken.keys) ?roster[key]]
      ..sort((a, b) => a.$1.compareTo(b.$1));
    return [
      for (final (_, member) in members) _memberCandidate(member),
      ...chips,
    ];
  }

  List<Object?> get signature => [
    sources.viewer,
    for (final chip in chips) ...[chip.pubkey.toLowerCase(), chip.label],
    ..._taken.values,
    for (final owner in _outsideOwners) ...[
      owner,
      sources.verifiedNames[owner],
      sources.profiles[owner]?.displayName,
    ],
  ];
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
/// them from nothing. Empty without a chip, before any member is read.
@visibleForTesting
Map<String, String> mentionChipQualifiers({
  required IdentityNameSources sources,
  required List<ChannelMember> members,
  required Map<String, UserProfile> userCache,
  required Map<String, String> agentOwners,
  required List<MentionCandidate> chips,
}) => MentionChipLabeller().labels(
  sources: sources,
  members: members,
  userCache: userCache,
  agentOwners: agentOwners,
  chips: chips,
);

/// Chip labels across changes to the naming facts, the roster and the
/// chips, as the composer keeps them.
///
/// A draft without a chip reads no member. With a chip:
///
/// - The same chips and inputs return the last labels.
/// - The same chips and roster list with newer naming facts compare the
///   facts with the last ones ([_ChipNaming.standsAfter]). A change that
///   cannot touch a chip's label reads no member and the labels stand.
/// - Anything else walks the roster once for the chips' namesakes and
///   names only those; when the walk finds the same context with the same
///   facts, the labels stand and nothing is named.
///
/// The roster is indexed by key once per list.
@visibleForTesting
class MentionChipLabeller {
  (List<ChannelMember>, _QualifierRoster)? _indexed;
  ({
    List<Object?> inputs,
    String chips,
    _ChipNaming naming,
    List<Object?> signature,
    Map<String, String> qualifiers,
  })?
  _named;

  /// See [MentionChipLabeller].
  Map<String, String> labels({
    required IdentityNameSources sources,
    required List<ChannelMember> members,
    required Map<String, UserProfile> userCache,
    required Map<String, String> agentOwners,
    required List<MentionCandidate> chips,
  }) {
    if (chips.isEmpty) return const {};
    final inputs = [sources, members, userCache, agentOwners];
    final key = [
      for (final chip in chips)
        '${chip.pubkey.toLowerCase()} ${chip.isAgent} ${chip.ownerPubkey} '
            '${chip.displayName}\n${chip.label}',
    ].join('\n\n');
    final last = _named;
    if (last != null && last.chips == key) {
      if (Iterable<int>.generate(
        inputs.length,
      ).every((i) => identical(last.inputs[i], inputs[i]))) {
        return last.qualifiers;
      }
      if (identical(last.inputs[1], members)) {
        final stands = _ChipNaming.standsAfter(
          last.naming,
          sources,
          userCache,
          agentOwners,
        );
        if (stands != null) {
          _named = (
            inputs: inputs,
            chips: key,
            naming: stands,
            signature: last.signature,
            qualifiers: last.qualifiers,
          );
          return last.qualifiers;
        }
      }
    }
    if (!identical(_indexed?.$1, members)) {
      _indexed = (members, _qualifierRoster(members));
    }
    final naming = _ChipNaming(
      sources,
      _indexed!.$2,
      userCache,
      agentOwners,
      chips,
    )..walk();
    final signature = naming.signature;
    final qualifiers = last != null && listEquals(last.signature, signature)
        ? last.qualifiers
        : _namedChipQualifiers(sources, naming.candidates, chips);
    _named = (
      inputs: inputs,
      chips: key,
      naming: naming,
      signature: signature,
      qualifiers: qualifiers,
    );
    return qualifiers;
  }
}

/// Keeps the editor's namesake chip labels current: a chip whose name
/// another candidate shares reads `Sam · a1b2`, the chooser's label, while
/// the draft text stays `@Sam`.
///
/// The labels come from one [MentionChipLabeller]: a draft without a chip
/// reads no member, and with a chip a profile, verified name or owner
/// change that cannot touch a chip's label reads none either. An edit that
/// leaves the chips as they were, or takes them away and puts them back,
/// reuses the last labels.
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
  final labeller = useMemoized(MentionChipLabeller.new);
  final current = useRef<List<Object?>?>(null);
  useEffect(() {
    final inputs = current.value = [sources, roster, userCache, agentOwners];
    void update() {
      // The effect these inputs replaced still listens while this one
      // starts, and would label the chips from the facts it was given.
      if (!identical(current.value, inputs)) return;
      controller.setMentionQualifiers(
        labeller.labels(
          sources: sources,
          members: roster,
          userCache: userCache,
          agentOwners: agentOwners ?? const {},
          chips: [for (final mention in controller.mentions) mention.candidate],
        ),
      );
    }

    update();
    controller.addListener(update);
    return () => controller.removeListener(update);
  }, [controller, sources, roster, userCache, agentOwners]);
}
