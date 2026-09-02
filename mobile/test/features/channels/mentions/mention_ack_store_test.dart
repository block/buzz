import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:buzz/features/channels/mentions/mention_ack_store.dart';
import 'package:buzz/shared/relay/relay.dart';

// NIP-MR mobile ack consumption. Mirrors the desktop semantics in
// desktop/src/features/agents/pendingMentionAckStore.ts, minus the `silent`
// timeout outcome, which is deliberately out of scope for this slice.

const _mentionId = 'mention-event-id';
const _agentPubkey =
    'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa';
const _otherPubkey =
    'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb';
const _authorPubkey =
    'cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc';
const _channelId = 'channel-1';

/// Builds an ack with the exact tag shape the ACP publisher emits.
/// See crates/buzz-acp/src/pool.rs build_mention_ack_event: h, e, p, status,
/// and an optional reason.
NostrEvent _ack({
  String signer = _agentPubkey,
  String targetEventId = _mentionId,
  String? status = mentionAckStatusAccepted,
  String? reason,
  String id = 'ack-1',
  int kind = EventKind.agentMentionAck,
}) {
  return NostrEvent(
    id: id,
    pubkey: signer,
    createdAt: 1000,
    kind: kind,
    tags: [
      ['h', _channelId],
      ['e', targetEventId],
      // `p` is the mention's AUTHOR, not the agent. Attribution must never
      // come from this tag.
      ['p', _authorPubkey],
      if (status != null) ['status', status],
      if (reason != null) ['reason', reason],
    ],
    content: '',
    sig: 'sig',
  );
}

MentionAckNotifier _notifier(ProviderContainer container) =>
    container.read(mentionAckStoreProvider.notifier);

ProviderContainer _container() {
  final container = ProviderContainer();
  addTearDown(container.dispose);
  return container;
}

void main() {
  group('parseMentionAckOutcome', () {
    test('accepted ack is attributed to the signer, never the p tag', () {
      final outcome = parseMentionAckOutcome(_ack())!;

      expect(outcome.status, MentionAckStatus.accepted);
      expect(outcome.agentPubkey, _agentPubkey);
      expect(outcome.agentPubkey, isNot(_authorPubkey));
      expect(outcome.reason, isNull);
    });

    test('declined ack carries its reason', () {
      final outcome = parseMentionAckOutcome(
        _ack(status: mentionAckStatusDeclined, reason: 'queue_full'),
      )!;

      expect(outcome.status, MentionAckStatus.declined);
      expect(outcome.reason, 'queue_full');
    });

    test('unknown, absent and empty status values are ignored', () {
      // The security invariant: an unrecognized status must never be coerced
      // into a decline.
      expect(parseMentionAckOutcome(_ack(status: 'maybe')), isNull);
      expect(parseMentionAckOutcome(_ack(status: null)), isNull);
      expect(parseMentionAckOutcome(_ack(status: '')), isNull);
      expect(parseMentionAckOutcome(_ack(status: 'ACCEPTED')), isNull);
    });

    test('non-44102 kinds are not acks', () {
      expect(
        parseMentionAckOutcome(_ack(kind: EventKind.streamMessageV2)),
        isNull,
      );
    });

    test('an untrusted reason is length-clamped', () {
      final outcome = parseMentionAckOutcome(
        _ack(
          status: mentionAckStatusDeclined,
          reason: 'x' * (mentionAckReasonMaxLength + 500),
        ),
      )!;

      expect(outcome.reason!.length, mentionAckReasonMaxLength);
    });

    test('a blank reason becomes null rather than an empty line', () {
      final outcome = parseMentionAckOutcome(
        _ack(status: mentionAckStatusDeclined, reason: '   '),
      )!;

      expect(outcome.reason, isNull);
    });
  });

  group('applyAck', () {
    test('accepted ack is visible to the tagged mention', () {
      final container = _container();

      expect(_notifier(container).applyAck(_ack()), isTrue);

      final state = container.read(mentionAckStoreProvider);
      expect(state.isAccepted(_mentionId, [_agentPubkey]), isTrue);
      expect(state.declines(_mentionId, [_agentPubkey]), isEmpty);
    });

    test('declined ack surfaces the reason', () {
      final container = _container();

      _notifier(container).applyAck(
        _ack(status: mentionAckStatusDeclined, reason: 'agent is offline'),
      );

      final declines = container.read(mentionAckStoreProvider).declines(
        _mentionId,
        [_agentPubkey],
      );
      expect(declines.single.reason, 'agent is offline');
      expect(
        container.read(mentionAckStoreProvider).isAccepted(_mentionId, [
          _agentPubkey,
        ]),
        isFalse,
      );
    });

    test('double delivery is idempotent', () {
      final container = _container();

      expect(_notifier(container).applyAck(_ack()), isTrue);
      // Same ack redelivered — both live subscriptions can deliver it.
      expect(_notifier(container).applyAck(_ack()), isFalse);
      // Same verdict, different event id: still the same fact.
      expect(_notifier(container).applyAck(_ack(id: 'ack-2')), isFalse);

      expect(
        container.read(mentionAckStoreProvider).outcomesFor(_mentionId, [
          _agentPubkey,
        ]),
        hasLength(1),
      );
    });

    test('an ack whose signer was not tagged in the mention is ignored', () {
      final container = _container();

      // A well-formed ack from an arbitrary member. The relay is pure fan-out
      // and cannot check agent-ness, so this must be inert.
      _notifier(container).applyAck(_ack(signer: _otherPubkey));

      final state = container.read(mentionAckStoreProvider);
      expect(state.outcomesFor(_mentionId, [_agentPubkey]), isEmpty);
      expect(state.isAccepted(_mentionId, [_agentPubkey]), isFalse);
      // It is only visible to a mention that actually tagged that signer.
      expect(state.isAccepted(_mentionId, [_otherPubkey]), isTrue);
    });

    test('an ack with no e tag is dropped', () {
      final container = _container();

      expect(_notifier(container).applyAck(_ack(targetEventId: '')), isFalse);
    });

    test('an accept from any tagged agent outranks another agent decline', () {
      final container = _container();

      _notifier(
        container,
      ).applyAck(_ack(status: mentionAckStatusDeclined, reason: 'busy'));
      _notifier(container).applyAck(_ack(signer: _otherPubkey));

      final state = container.read(mentionAckStoreProvider);
      final tagged = [_agentPubkey, _otherPubkey];
      expect(state.isAccepted(_mentionId, tagged), isTrue);
      // Nothing to warn about once someone is taking the turn.
      expect(state.declines(_mentionId, tagged), isEmpty);
    });

    test('a later verdict from the same agent replaces the earlier one', () {
      final container = _container();

      _notifier(container).applyAck(_ack());
      expect(
        _notifier(container).applyAck(
          _ack(status: mentionAckStatusDeclined, reason: 'changed my mind'),
        ),
        isTrue,
      );

      final state = container.read(mentionAckStoreProvider);
      expect(state.isAccepted(_mentionId, [_agentPubkey]), isFalse);
      expect(
        state.declines(_mentionId, [_agentPubkey]).single.reason,
        'changed my mind',
      );
    });

    test('acks for other mentions do not leak across event ids', () {
      final container = _container();

      _notifier(container).applyAck(_ack(targetEventId: 'someone-else'));

      final state = container.read(mentionAckStoreProvider);
      expect(state.outcomesFor(_mentionId, [_agentPubkey]), isEmpty);
      expect(state.isAccepted(_mentionId, [_agentPubkey]), isFalse);
    });

    test('a mention that tagged nobody can never show an outcome', () {
      final container = _container();

      _notifier(container).applyAck(_ack());

      expect(
        container
            .read(mentionAckStoreProvider)
            .outcomesFor(_mentionId, const []),
        isEmpty,
      );
    });

    test('signer matching is case-insensitive on both sides', () {
      final container = _container();

      _notifier(container).applyAck(_ack(signer: _agentPubkey.toUpperCase()));

      expect(
        container.read(mentionAckStoreProvider).isAccepted(_mentionId, [
          _agentPubkey.toUpperCase(),
        ]),
        isTrue,
      );
    });

    test('tracked mentions are bounded so acks cannot grow memory', () {
      final container = _container();

      for (var i = 0; i < mentionAckMaxTrackedEvents + 25; i++) {
        _notifier(
          container,
        ).applyAck(_ack(targetEventId: 'mention-$i', id: 'ack-$i'));
      }

      final state = container.read(mentionAckStoreProvider);
      expect(state.byEventId.length, mentionAckMaxTrackedEvents);
      // Oldest evicted, newest retained.
      expect(state.byEventId.containsKey('mention-0'), isFalse);
      expect(
        state.byEventId.containsKey(
          'mention-${mentionAckMaxTrackedEvents + 24}',
        ),
        isTrue,
      );
    });

    test('reset clears state, mirroring the community-switch discipline', () {
      final container = _container();

      _notifier(container).applyAck(_ack());
      expect(container.read(mentionAckStoreProvider).byEventId, isNotEmpty);

      _notifier(container).reset();

      final state = container.read(mentionAckStoreProvider);
      expect(state.byEventId, isEmpty);
      expect(state.isAccepted(_mentionId, [_agentPubkey]), isFalse);
    });
  });

  group('kind wiring', () {
    test('44102 is subscribed and treated as an overlay, never a row', () {
      // Subscribed, so acks reach the client at all.
      expect(EventKind.channelEventKinds, contains(EventKind.agentMentionAck));
      // Aux, so it overlays instead of rendering as a timeline row.
      expect(
        EventKind.channelAuxEventKinds,
        contains(EventKind.agentMentionAck),
      );
      // Never a visible message row.
      expect(
        EventKind.channelMessageEventKinds,
        isNot(contains(EventKind.agentMentionAck)),
      );
      expect(
        EventKind.channelTimelineContentKinds,
        isNot(contains(EventKind.agentMentionAck)),
      );
    });
  });
}
