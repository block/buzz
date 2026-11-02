import 'dart:convert';

import 'package:flutter/foundation.dart';
import '../../shared/crypto/event_signing.dart';
import '../../shared/crypto/nip_oa.dart';
import '../../shared/relay/relay_author_provider.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/relay/relay.dart';
import '../../shared/mentions/agent_identity_provider.dart';
import 'channel.dart';
import 'channel_management_provider.dart';
import 'channels_provider.dart';

/// Fresh, signed metadata and roster used together for access decisions.
class ChannelAccessSnapshot {
  final NostrEvent metadata;
  final NostrEvent roster;
  const ChannelAccessSnapshot(this.metadata, this.roster);
  ChannelData get data => ChannelData.fromEvent(metadata);
  String? role(String? pubkey) {
    final matches = membersFromEvent(roster)
        .where((member) => member.pubkey.toLowerCase() == pubkey?.toLowerCase())
        .toList();
    return matches.length == 1 ? matches.single.role : null;
  }

  bool contains(String? pubkey) => membersFromEvent(
    roster,
  ).any((member) => member.pubkey.toLowerCase() == pubkey?.toLowerCase());
  bool canEdit(String? actor) =>
      !data.isArchived &&
      data.channelType != 'dm' &&
      const {'owner', 'admin'}.contains(role(actor));
}

/// Submitted intent retained for readback only after an uncertain outcome.
class ChannelAccessAttempt {
  final String baseMetadataId;
  final String baseRosterId;
  final int kind;
  final List<List<String>> tags;
  final String? target;
  final String? role;
  final bool remove;
  final bool addition;
  final String? visibility;
  final String? joinRole;
  final List<String>? labels;
  const ChannelAccessAttempt({
    required this.baseMetadataId,
    required this.baseRosterId,
    required this.kind,
    required this.tags,
    this.target,
    this.role,
    this.remove = false,
    this.addition = false,
    this.visibility,
    this.joinRole,
    this.labels,
  });

  bool matches(ChannelAccessSnapshot snapshot) {
    if (target != null) {
      return remove
          ? !snapshot.contains(target)
          : snapshot.role(target) == role;
    }
    final data = snapshot.data;
    return (visibility == null || data.visibility == visibility) &&
        (joinRole == null || data.joinRole == joinRole) &&
        (labels == null || listEquals(data.labels, labels));
  }
}

/// Owns pending access writes within one account/community session. A lost
/// response or mismatching readback never turns Retry into a second write.
class ChannelAccessController
    extends Notifier<Map<String, ChannelAccessAttempt>> {
  bool _valid = true;
  Object _scope = Object();
  final _busy = <String>{};
  late RelaySessionNotifier _session;
  late SignedEventRelay _writer;
  String? _actor;
  late String _baseUrl;

  @override
  Map<String, ChannelAccessAttempt> build() {
    final config = ref.watch(relayConfigProvider);
    _baseUrl = config.baseUrl;
    // Keep discovery alive without making its refresh discard pending writes.
    // load() observes errors and reads the current discovery future each time.
    ref.listen(relayAuthorProvider(_baseUrl), (_, _) {});
    _actor = ref.watch(currentPubkeyProvider)?.toLowerCase();
    _session = ref.watch(relaySessionProvider.notifier);
    _writer = SignedEventRelay(session: _session, nsec: config.nsec);
    _scope = Object();
    _busy.clear();
    _valid = true;
    ref.onDispose(() => _valid = false);
    return {};
  }

  void _check([Object? scope]) {
    if (!_valid || (scope != null && !identical(scope, _scope))) {
      throw StateError('Your account or community changed. Open access again.');
    }
  }

  /// Loads only this channel. Failed or absent reads cannot grant authority.
  Future<ChannelAccessSnapshot> load(String id) async {
    _check();
    final scope = _scope;
    final session = _session;
    final discovery = relayAuthorProvider(_baseUrl);
    if (ref.read(discovery).hasError) ref.invalidate(discovery);
    final author = await ref
        .read(discovery.future)
        .timeout(const Duration(seconds: 8));
    _check(scope);
    if (author == null) {
      throw StateError(
        'The community signer could not be confirmed. Reload access.',
      );
    }
    Future<NostrEvent> read(int kind) async {
      final events = await session
          .fetchHistory(
            NostrFilter(
              kinds: [kind],
              authors: [author],
              tags: {
                '#d': [id],
              },
              limit: 1,
            ),
          )
          .timeout(const Duration(seconds: 20));
      _check(scope);
      final event =
          events
              .where(
                (event) =>
                    event.kind == kind &&
                    event.pubkey == author &&
                    event.getTagValue('d') == id,
              )
              .firstOrNull ??
          (throw StateError('Current channel access could not be confirmed.'));
      final encoded = jsonEncode(event.toJson());
      verifiedNostrEventFromJson(encoded);
      _check(scope);
      return event;
    }

    final events = await Future.wait([read(39000), read(39002)]);
    _check(scope);
    return ChannelAccessSnapshot(events[0], events[1]);
  }

  /// Fresh publication check. A Guest default never replaces an explicit role.
  Future<void> requireContribution(String id) async {
    final scope = _scope;
    final snapshot = await load(id);
    _check(scope);
    if (_actor != _writer.pubkey?.toLowerCase() ||
        snapshot.data.isArchived ||
        !channelRoleCanContribute(snapshot.role(_actor))) {
      throw StateError(
        'Read-only access. Ask an owner or admin to participate.',
      );
    }
  }

  /// Ordinary own-message mutations require contribution. A joined owner of an
  /// agent retains the relay's separate edit/delete exception, even as Guest.
  Future<void> requireMessageManagement(String id, String eventId) async {
    final scope = _scope;
    final snapshot = await load(id);
    _check(scope);
    if (snapshot.data.isArchived ||
        _actor != _writer.pubkey?.toLowerCase() ||
        snapshot.role(_actor) == null) {
      throw StateError('Message changes are unavailable. Reload access.');
    }
    final events = await _session.fetchHistory(
      NostrFilter(
        ids: [eventId],
        kinds: const [
          EventKind.streamMessage,
          EventKind.streamMessageV2,
          EventKind.forumPost,
          EventKind.forumComment,
        ],
        tags: {
          '#h': [id],
        },
        limit: 1,
      ),
    );
    _check(scope);
    final event =
        events
            .where((event) => event.id == eventId && event.channelId == id)
            .firstOrNull ??
        (throw StateError('The message could not be confirmed.'));
    verifiedNostrEventFromJson(jsonEncode(event.toJson()));
    if (event.pubkey == _actor &&
        channelRoleCanContribute(snapshot.role(_actor))) {
      return;
    }
    final profiles = await _session.fetchHistory(
      NostrFilters.profilesBatch([event.pubkey]),
    );
    _check(scope);
    final matching =
        profiles
            .where(
              (profile) => profile.kind == 0 && profile.pubkey == event.pubkey,
            )
            .toList()
          ..sort(
            (a, b) => a.createdAt == b.createdAt
                ? a.id.compareTo(b.id)
                : b.createdAt.compareTo(a.createdAt),
          );
    if (matching.isEmpty ||
        verifiedProfileOaOwnerPubkey(matching.first) != _actor) {
      throw StateError('Current message ownership could not be confirmed.');
    }
  }

  /// Grants an absent identity once. Existing membership is never rewritten by
  /// an invitation retry, including an agent already promoted or demoted.
  Future<void> addMember(
    String id,
    String target,
    String role, {
    void Function()? beforeDispatch,
  }) async {
    if (state.containsKey(id)) await checkSave(id);
    final base = await load(id);
    if (base.contains(target)) return;
    await _commit(
      id,
      base,
      ChannelAccessAttempt(
        baseMetadataId: base.metadata.id,
        baseRosterId: base.roster.id,
        kind: 9000,
        tags: [
          ['h', id],
          ['p', target.toLowerCase()],
          ['role', role],
        ],
        target: target.toLowerCase(),
        role: role,
        addition: true,
      ),
      beforeDispatch: beforeDispatch,
    );
  }

  /// Changes audience and admission together; Private preserves the default.
  Future<ChannelAccessSnapshot> setAccess(
    String id,
    ChannelAccessSnapshot base,
    String choice,
  ) async {
    final data = base.data;
    if (!const {'private', 'public', 'view-only'}.contains(choice) ||
        (data.labels.contains('buzz.solo') && choice == 'public')) {
      throw StateError(
        'Public Solo access is unavailable: anyone joining could direct your agents.',
      );
    }
    final visibility = choice == 'private' ? 'private' : 'open';
    final role = choice == 'private'
        ? data.joinRole
        : choice == 'view-only'
        ? 'guest'
        : 'member';
    return _commit(
      id,
      base,
      ChannelAccessAttempt(
        baseMetadataId: base.metadata.id,
        baseRosterId: base.roster.id,
        kind: 9002,
        tags: [
          ['h', id],
          if (visibility != data.visibility) ['visibility', visibility],
          if (role != data.joinRole) ['join_role', role],
        ],
        visibility: visibility,
        joinRole: role,
      ),
    );
  }

  /// Removes the Solo label and saves the selected policy, preserving lifetime.
  Future<ChannelAccessSnapshot> convert(
    String id,
    ChannelAccessSnapshot base, {
    String? access,
  }) async {
    if (!base.data.labels.contains('buzz.solo')) {
      throw StateError('This chat is already a channel.');
    }
    final choice =
        access ??
        (base.data.visibility == 'private'
            ? 'private'
            : base.data.joinRole == 'guest'
            ? 'view-only'
            : 'public');
    if (!const {'private', 'view-only', 'public'}.contains(choice)) {
      throw ArgumentError.value(access, 'access');
    }
    final visibility = choice == 'private' ? 'private' : 'open';
    final joinRole = choice == 'private'
        ? base.data.joinRole
        : choice == 'view-only'
        ? 'guest'
        : 'member';
    final labels = base.data.labels
        .where((label) => label != 'buzz.solo')
        .toList();
    return _commit(
      id,
      base,
      ChannelAccessAttempt(
        baseMetadataId: base.metadata.id,
        baseRosterId: base.roster.id,
        kind: 9002,
        tags: [
          ['h', id],
          if (visibility != base.data.visibility) ['visibility', visibility],
          if (joinRole != base.data.joinRole) ['join_role', joinRole],
          if (labels.isEmpty)
            ['t', '']
          else
            for (final label in labels) ['t', label],
        ],
        labels: labels,
        visibility: visibility,
        joinRole: joinRole,
      ),
    );
  }

  /// Changes one existing person's role or removes them, preserving Owner/self.
  Future<ChannelAccessSnapshot> changeMember(
    String id,
    ChannelAccessSnapshot base,
    String target, {
    String? role,
  }) async {
    if (role != null &&
        !const {'admin', 'member', 'guest', 'bot'}.contains(role)) {
      throw ArgumentError.value(role, 'role');
    }
    return _commit(
      id,
      base,
      ChannelAccessAttempt(
        baseMetadataId: base.metadata.id,
        baseRosterId: base.roster.id,
        kind: role == null ? 9001 : 9000,
        tags: [
          ['h', id],
          ['p', target.toLowerCase()],
          if (role != null) ['role', role],
        ],
        target: target.toLowerCase(),
        role: role,
        remove: role == null,
      ),
    );
  }

  Future<void> _authorize(
    ChannelAccessSnapshot snapshot,
    ChannelAccessAttempt intent,
  ) async {
    if (_actor != _writer.pubkey?.toLowerCase()) {
      throw StateError('The signing identity changed. Reload access.');
    }
    if (intent.addition) {
      final actorRole = snapshot.role(_actor);
      final ordinaryPreview =
          actorRole == null &&
          !snapshot.contains(_actor) &&
          snapshot.data.visibility == 'open' &&
          snapshot.data.joinRole == 'member' &&
          !snapshot.data.labels.contains('buzz.solo');
      final canAdd =
          !snapshot.data.isArchived &&
          snapshot.data.channelType != 'dm' &&
          (channelRoleCanContribute(actorRole) || ordinaryPreview);
      if (!canAdd ||
          snapshot.contains(intent.target) ||
          !const {'admin', 'member', 'guest', 'bot'}.contains(intent.role) ||
          (intent.role == 'admin' && !snapshot.canEdit(_actor))) {
        throw StateError(
          'This invitation is no longer available. Reload access.',
        );
      }
      return;
    }
    final target = intent.target;
    if (!snapshot.canEdit(_actor)) {
      final ownRemoval =
          intent.remove &&
          target != null &&
          target != _actor &&
          !snapshot.data.isArchived &&
          snapshot.data.channelType != 'dm' &&
          !snapshot.data.labels.contains('buzz.solo') &&
          const {
            'owner',
            'admin',
            'member',
            'guest',
            'bot',
          }.contains(snapshot.role(_actor)) &&
          const {
            'admin',
            'member',
            'guest',
            'bot',
          }.contains(snapshot.role(target));
      if (!ownRemoval) {
        throw StateError('Only an owner or admin can change access.');
      }
      final profiles = await _session.fetchHistory(
        NostrFilters.profilesBatch([target]),
      );
      final matching =
          profiles
              .where((event) => event.kind == 0 && event.pubkey == target)
              .toList()
            ..sort(
              (a, b) => a.createdAt == b.createdAt
                  ? a.id.compareTo(b.id)
                  : b.createdAt.compareTo(a.createdAt),
            );
      if (matching.isEmpty ||
          verifiedProfileOaOwnerPubkey(matching.first) != _actor) {
        throw StateError('Current agent ownership could not be confirmed.');
      }
    }
    if (target != null &&
        !intent.remove &&
        snapshot.data.labels.contains('buzz.solo')) {
      throw StateError('Individual roles cannot be changed in a Solo chat.');
    }
    if (target != null &&
        (target == _actor ||
            snapshot.role(target) == null ||
            !const {
              'admin',
              'member',
              'guest',
              'bot',
            }.contains(snapshot.role(target)))) {
      throw StateError('This member cannot be changed. Reload access.');
    }
  }

  Future<ChannelAccessSnapshot> _commit(
    String id,
    ChannelAccessSnapshot base,
    ChannelAccessAttempt intent, {
    void Function()? beforeDispatch,
  }) async {
    _check();
    final scope = _scope;
    if (state.containsKey(id) || !_busy.add(id)) {
      throw StateError('Check the unfinished access save first.');
    }
    try {
      await _authorize(base, intent);
      _check(scope);
      final fresh = await load(id);
      _check(scope);
      await _authorize(fresh, intent);
      _check(scope);
      if (fresh.metadata.id != base.metadata.id ||
          fresh.roster.id != base.roster.id) {
        throw StateError(
          'Channel access changed. Reload access and review your choice.',
        );
      }
      _check(scope);
      beforeDispatch?.call();
      // Mark before dispatch. A synchronous signing failure is unsent; a signed
      // request without a definitive rejection may already have landed.
      var signed = false;
      try {
        await _writer
            .submit(
              kind: intent.kind,
              content: '',
              tags: intent.tags,
              onSigned: (_) {
                signed = true;
                state = {...state, id: intent};
              },
            )
            .timeout(const Duration(seconds: 20));
      } catch (error) {
        _check(scope);
        if (!signed || error is RelayEventRejected) {
          state = {...state}..remove(id);
        }
        rethrow;
      }
      return await checkSave(id);
    } finally {
      if (identical(scope, _scope)) _busy.remove(id);
    }
  }

  /// Reconciles uncertain publication using reads only, including after reopen.
  Future<ChannelAccessSnapshot> checkSave(String id) async {
    _check();
    final scope = _scope;
    final intent = state[id];
    if (intent == null) return load(id);
    final snapshot = await load(id);
    _check(scope);
    if (!identical(state[id], intent)) {
      throw StateError('The access save changed. Review its current status.');
    }
    if (!intent.matches(snapshot)) {
      if (snapshot.metadata.id != intent.baseMetadataId ||
          snapshot.roster.id != intent.baseRosterId) {
        _finishAttempt(id);
        throw StateError(
          'Access changed elsewhere; review the current setting.',
        );
      }
      throw StateError(
        'Save is not confirmed. Check save status before making another change.',
      );
    }
    _finishAttempt(id);
    return snapshot;
  }

  /// Abandons local recovery after explicit consent, without resending a write.
  /// The previous request may still arrive; fresh state must be reviewed again.
  Future<ChannelAccessSnapshot> discardSave(
    String id,
    ChannelAccessAttempt expected,
  ) async {
    _check();
    final scope = _scope;
    if (!identical(state[id], expected) || !_busy.add(id)) {
      throw StateError(
        'The access save changed or is still running. Check its status.',
      );
    }
    try {
      final snapshot = await load(id);
      _check(scope);
      if (!identical(state[id], expected)) {
        throw StateError('The access save changed. Review its current status.');
      }
      _finishAttempt(id);
      return snapshot;
    } finally {
      if (identical(scope, _scope)) _busy.remove(id);
    }
  }

  void _finishAttempt(String id) {
    state = {...state}..remove(id);
    ref.invalidate(channelDetailsProvider(id));
    ref.invalidate(channelMembersProvider(id));
    ref.invalidate(channelBotPubkeysProvider(id));
    ref.invalidate(channelsProvider);
  }
}

final channelAccessProvider =
    NotifierProvider<
      ChannelAccessController,
      Map<String, ChannelAccessAttempt>
    >(ChannelAccessController.new);
