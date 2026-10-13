/// NIP-MR: agent acknowledgements (kind:44102) for a mention.
///
/// An agent harness publishes a receipt the moment it decides what to do with a
/// mention — `accepted` when a turn is coming, `declined` with a reason when it
/// knowingly will not act. Desktop consumes these
/// (`desktop/src/features/agents/pendingMentionAckStore.ts`); mobile did not, so
/// a phone user could not tell a decline from a delay.
///
/// Deliberate scope cut versus desktop: there is NO `silent` outcome and no
/// client-side timer. Mobile suspends and disconnects often, so a pending timer
/// would fire false "nobody picked this up" verdicts after resume. This store
/// holds only *received facts*: an ack either arrived or it did not. A missing
/// ack renders exactly as today, never as a false decline.
///
/// Live-only in memory and community-scoped, matching desktop's
/// `resetPendingMentionAckStore()` discipline: the provider below rebuilds (and
/// therefore empties) when the active community changes.
library;

import 'package:flutter/foundation.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../../shared/relay/relay.dart';

/// Maximum characters retained from an untrusted `reason` tag.
///
/// The reason is attacker-controlled text from an arbitrary relay member, so it
/// is clamped here — at the parse boundary — rather than trusting every future
/// render site to bound it.
const int mentionAckReasonMaxLength = 200;

/// Upper bound on tracked mention event ids.
///
/// Acks for *other* people's mentions are recorded too, because this store
/// cannot know which messages are the local identity's without coupling itself
/// to the timeline. That makes the map attacker-growable, so it is bounded and
/// evicts in insertion order.
const int mentionAckMaxTrackedEvents = 512;

/// The status values this client understands. Anything else is ignored rather
/// than being coerced into a decline.
const String mentionAckStatusAccepted = 'accepted';
const String mentionAckStatusDeclined = 'declined';

enum MentionAckStatus { accepted, declined }

/// One agent's verdict on one mention.
@immutable
class MentionAckOutcome {
  /// The pubkey that SIGNED the ack. Never a `p` tag value.
  final String agentPubkey;
  final MentionAckStatus status;

  /// Untrusted, already length-clamped. Render as plain text only — never as
  /// markdown and never as a link.
  final String? reason;

  const MentionAckOutcome({
    required this.agentPubkey,
    required this.status,
    this.reason,
  });

  bool get isAccepted => status == MentionAckStatus.accepted;
  bool get isDeclined => status == MentionAckStatus.declined;

  @override
  bool operator ==(Object other) =>
      other is MentionAckOutcome &&
      other.agentPubkey == agentPubkey &&
      other.status == status &&
      other.reason == reason;

  @override
  int get hashCode => Object.hash(agentPubkey, status, reason);

  @override
  String toString() =>
      'MentionAckOutcome($agentPubkey, ${status.name}, reason: $reason)';
}

/// Immutable snapshot: mention event id -> signer pubkey -> outcome.
///
/// Keyed by signer so a double delivery of the same ack overwrites rather than
/// appends: idempotency falls out of the data shape instead of relying on
/// callers to deduplicate.
@immutable
class MentionAckState {
  final Map<String, Map<String, MentionAckOutcome>> byEventId;

  const MentionAckState({this.byEventId = const {}});

  /// Outcomes for [eventId], restricted to signers the mention actually tagged.
  ///
  /// This is the authorization boundary. The relay is pure fan-out and cannot
  /// check agent-ness, so ANY member can publish a well-formed ack for someone
  /// else's message. Requiring the signer to appear in [taggedPubkeys] makes
  /// such an ack inert. Callers pass the mention's own `p`/`mention` tags.
  List<MentionAckOutcome> outcomesFor(
    String eventId,
    Iterable<String> taggedPubkeys,
  ) {
    final outcomes = byEventId[eventId];
    if (outcomes == null || outcomes.isEmpty) return const [];

    final allowed = {
      for (final pubkey in taggedPubkeys) pubkey.trim().toLowerCase(),
    }..remove('');
    if (allowed.isEmpty) return const [];

    return [
      for (final entry in outcomes.entries)
        if (allowed.contains(entry.key)) entry.value,
    ];
  }

  /// Whether any tagged agent accepted. An accept outranks a decline: if one
  /// agent is taking the turn, the mention was answered.
  bool isAccepted(String eventId, Iterable<String> taggedPubkeys) =>
      outcomesFor(eventId, taggedPubkeys).any((o) => o.isAccepted);

  /// Declines, surfaced only when nothing accepted.
  List<MentionAckOutcome> declines(
    String eventId,
    Iterable<String> taggedPubkeys,
  ) {
    final outcomes = outcomesFor(eventId, taggedPubkeys);
    if (outcomes.any((o) => o.isAccepted)) return const [];
    return [
      for (final outcome in outcomes)
        if (outcome.isDeclined) outcome,
    ];
  }
}

/// Parse a kind:44102 event into an outcome, or null when it is not a
/// well-formed ack this client understands.
///
/// Attribution is to `event.pubkey` — the SIGNER — never to the `p` tag, which
/// carries the mention's author and is therefore trivially forgeable as an
/// identity claim.
MentionAckOutcome? parseMentionAckOutcome(NostrEvent event) {
  if (event.kind != EventKind.agentMentionAck) return null;

  final signer = event.pubkey.trim().toLowerCase();
  if (signer.isEmpty) return null;

  switch (event.getTagValue('status')) {
    case mentionAckStatusAccepted:
      return MentionAckOutcome(
        agentPubkey: signer,
        status: MentionAckStatus.accepted,
      );
    case mentionAckStatusDeclined:
      final raw = event.getTagValue('reason')?.trim();
      final reason = (raw == null || raw.isEmpty)
          ? null
          : (raw.length > mentionAckReasonMaxLength
                ? raw.substring(0, mentionAckReasonMaxLength)
                : raw);
      return MentionAckOutcome(
        agentPubkey: signer,
        status: MentionAckStatus.declined,
        reason: reason,
      );
    default:
      // Unknown or absent status: ignored, NOT rendered as a decline.
      return null;
  }
}

/// The mention event id an ack refers to, from its `e` tag.
String? mentionAckTargetEventId(NostrEvent event) {
  final target = event.getTagValue('e')?.trim();
  return (target == null || target.isEmpty) ? null : target;
}

class MentionAckNotifier extends Notifier<MentionAckState> {
  @override
  MentionAckState build() {
    // Community-scoped: the relay config rebuilds on community switch, which
    // drops every ack recorded against the previous community's identities.
    ref.watch(relayConfigProvider);
    return const MentionAckState();
  }

  /// Apply an incoming ack. Returns true when state changed.
  ///
  /// Idempotent by construction: re-applying the same ack produces an equal
  /// outcome for the same signer key and is dropped as a no-op.
  bool applyAck(NostrEvent event) {
    final eventId = mentionAckTargetEventId(event);
    if (eventId == null) return false;

    final outcome = parseMentionAckOutcome(event);
    if (outcome == null) return false;

    final existing = state.byEventId[eventId];
    if (existing != null && existing[outcome.agentPubkey] == outcome) {
      return false;
    }

    final next = <String, Map<String, MentionAckOutcome>>{
      ...state.byEventId,
      eventId: {...?existing, outcome.agentPubkey: outcome},
    };

    // Bounded: evict oldest insertions first. Map literals preserve insertion
    // order in Dart, and an updated key keeps its original position, so a
    // long-lived conversation cannot be made to grow without limit.
    if (next.length > mentionAckMaxTrackedEvents) {
      final surplus = next.length - mentionAckMaxTrackedEvents;
      for (final stale in next.keys.take(surplus).toList()) {
        next.remove(stale);
      }
    }

    state = MentionAckState(byEventId: next);
    return true;
  }

  /// Explicit reset. The provider already empties on community switch; this
  /// exists for tests and for any future identity-change path.
  void reset() => state = const MentionAckState();
}

final mentionAckStoreProvider =
    NotifierProvider<MentionAckNotifier, MentionAckState>(
      MentionAckNotifier.new,
    );
