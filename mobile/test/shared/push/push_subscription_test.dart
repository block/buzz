import 'dart:convert';

import 'package:buzz/shared/push/push_subscription.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  final me = 'a' * 64;
  const channelA = '123e4567-e89b-42d3-a456-426614174000';
  const channelB = '123e4567-e89b-42d3-a456-426614174001';

  List<String> channels(int count) => [
    for (var i = 0; i < count; i++)
      '00000000-0000-4000-8000-${i.toString().padLeft(12, '0')}',
  ];

  test(
    'authored mute capacity preserves every exclusion or omits the rule',
    () {
      final mutes = channels(351);
      final atLimit = buildDesiredBuzzPushSubscriptions(
        myPubkey: me,
        authoredThreadReplies: true,
        mutedChannelIds: mutes.take(350),
      ).singleWhere((s) => s.filter.authoredThreadReplies);
      expect(atLimit.ignore, hasLength(buzzPushMaxIgnoreFilters));
      expect(atLimit.ignore.skip(1).expand((f) => f.hTags!), mutes.take(350));

      final overLimit = buildDesiredBuzzPushSubscriptions(
        myPubkey: me,
        channelIds: [channelA],
        authoredThreadReplies: true,
        mutedChannelIds: mutes,
      );
      final baseline = buildDesiredBuzzPushSubscriptions(
        myPubkey: me,
        channelIds: [channelA],
        mutedChannelIds: mutes,
      );
      expect(
        buzzPushSubscriptionsFingerprint(overLimit),
        buzzPushSubscriptionsFingerprint(baseline),
      );
      expect(overLimit.last.filter.hTags, [channelA]);
    },
  );

  test('authored rule never consumes a required DM subscription slot', () {
    final dms = channels(750);
    final withRoom = buildDesiredBuzzPushSubscriptions(
      myPubkey: me,
      channelIds: dms.take(700),
      authoredThreadReplies: true,
    );
    expect(withRoom, hasLength(buzzPushMaxSubscriptions));
    expect(withRoom.any((s) => s.filter.authoredThreadReplies), isTrue);

    final full = buildDesiredBuzzPushSubscriptions(
      myPubkey: me,
      channelIds: dms,
      authoredThreadReplies: true,
    );
    expect(full, hasLength(buzzPushMaxSubscriptions));
    expect(full.any((s) => s.filter.authoredThreadReplies), isFalse);
    expect(full.skip(1).expand((s) => s.filter.hTags!), dms);
  });

  test(
    'authored replies opt in without enumerating roots and honor all mutes',
    () {
      final subscriptions = buildDesiredBuzzPushSubscriptions(
        myPubkey: me,
        authoredThreadReplies: true,
        authoredThreadRepliesSince: 1000,
        mutedChannelIds: [channelA],
      );
      final rule = subscriptions.singleWhere(
        (s) => s.filter.authoredThreadReplies,
      );
      expect(rule.filter.eTags, isNull);
      expect(rule.filter.since, 1000);
      expect(rule.ignore.first.authors, [me]);
      expect(rule.ignore.last.hTags, [channelA]);
      expect(rule.ignore.last.kinds, buzzPushEligibleKinds);
      expect(rule.suppress!.pTagsMax, buzzPushHellthreadParticipantLimit);
      expect(
        BuzzPushSubscription.fromJson(rule.toJson()).toJson(),
        rule.toJson(),
      );
      expect(
        buildDesiredBuzzPushSubscriptions(
          myPubkey: me,
        ).any((s) => s.filter.authoredThreadReplies),
        isFalse,
      );
    },
  );

  test(
    'desired subscription state round-trips with an accepted-state seam',
    () {
      final subscriptions = buildDesiredBuzzPushSubscriptions(
        myPubkey: me,
        channelIds: [channelB, channelA],
      );
      final state = BuzzPushLeaseSubscriptionState.desired(
        desired: subscriptions,
      );

      final decoded = BuzzPushLeaseSubscriptionState.fromJson(
        jsonDecode(jsonEncode(state.toJson())) as Map<String, dynamic>,
      );

      expect(decoded.authority, BuzzPushLeaseSubscriptionAuthority.desired);
      expect(decoded.accepted, isNull);
      expect(decoded.toJson(), state.toJson());
      expect(decoded.authoritative, hasLength(2));
      expect(decoded.authoritative.last.filter.hTags, [channelA, channelB]);
    },
  );

  test('accepted authority requires observed accepted subscriptions', () {
    final subscription = buildDesiredBuzzPushSubscriptions(myPubkey: me).single;

    expect(
      () => BuzzPushLeaseSubscriptionState.fromJson({
        'authority': 'accepted',
        'desired': [subscription.toJson()],
      }),
      throwsFormatException,
    );
  });

  test('persists accepted and reserved relay lease generations', () {
    final subscription = buildDesiredBuzzPushSubscriptions(myPubkey: me).single;
    final state =
        BuzzPushLeaseSubscriptionState.desired(desired: [subscription])
            .withAccepted(subscriptions: [subscription], generation: 9)
            .withReservedGeneration(10);

    final decoded = BuzzPushLeaseSubscriptionState.fromJson(state.toJson());
    expect(decoded.acceptedGeneration, 9);
    expect(decoded.generationCursor, 10);
    expect(decoded.toJson(), state.toJson());
  });

  test('a retry reserves beyond a committed but unrecorded generation', () {
    final subscription = buildDesiredBuzzPushSubscriptions(myPubkey: me).single;
    final accepted = BuzzPushLeaseSubscriptionState.desired(
      desired: [subscription],
    ).withAccepted(subscriptions: [subscription], generation: 4);
    final committed = accepted.withReservedGeneration(5);

    final recovered = BuzzPushLeaseSubscriptionState.fromJson(
      committed.toJson(),
    ).withReservedGeneration(6);

    expect(recovered.acceptedGeneration, 4);
    expect(recovered.generationCursor, 6);
  });

  test('builds aligned self and unmuted channel subscriptions', () {
    final subscriptions = buildDesiredBuzzPushSubscriptions(
      myPubkey: me.toUpperCase(),
      channelIds: [channelB, channelA],
      mutedChannelIds: [channelB, '123e4567-e89b-42d3-a456-426614174099'],
    );

    expect(subscriptions, hasLength(2));
    expect(subscriptions.first.filter.kinds, buzzPushSelfDirectedKinds);
    expect(subscriptions.first.filter.kinds, isNot(contains(7)));
    expect(subscriptions.first.filter.pTags, [me]);
    expect(subscriptions.last.filter.kinds, buzzPushChannelKinds);
    expect(subscriptions.last.filter.hTags, [channelA]);
    expect(subscriptions.first.ignore, hasLength(2));
    expect(subscriptions.first.ignore.first.kinds, buzzPushRenderableKinds);
    expect(subscriptions.first.ignore.last.hTags, [channelB]);
    expect(
      subscriptions.first.suppress?.pTagsMax,
      buzzPushHellthreadParticipantLimit,
    );
  });

  test('chunks channel subscriptions to relay limits deterministically', () {
    final channels = [
      for (var i = 0; i < 51; i++)
        '00000000-0000-4000-8000-${i.toString().padLeft(12, '0')}',
    ]..shuffle();

    final subscriptions = buildDesiredBuzzPushSubscriptions(
      myPubkey: me,
      channelIds: channels,
    );

    expect(subscriptions, hasLength(3));
    expect(subscriptions[1].filter.hTags, hasLength(50));
    expect(subscriptions[2].filter.hTags, hasLength(1));
    expect(
      subscriptions[1].filter.hTags,
      orderedEquals([...subscriptions[1].filter.hTags!]..sort()),
    );
  });

  test('rejects malformed and unsupported subscription fields', () {
    final valid = buildDesiredBuzzPushSubscriptions(
      myPubkey: me,
    ).single.toJson();

    expect(
      () => buildDesiredBuzzPushSubscriptions(
        myPubkey: me,
        channelIds: const ['not-a-channel'],
      ),
      throwsFormatException,
    );

    expect(
      () => BuzzPushSubscription.fromJson({...valid, 'unexpected': true}),
      throwsFormatException,
    );
    expect(
      () => BuzzPushSubscription.fromJson({
        'filter': {
          'kinds': [7],
          '#p': [me],
        },
        'class': 'default',
      }),
      throwsFormatException,
    );
    expect(
      () => BuzzPushSubscription.fromJson({
        'filter': {
          'kinds': [9],
          '#p': ['not-a-pubkey'],
        },
        'class': 'default',
      }),
      throwsFormatException,
    );
  });
}
