import 'dart:convert';
import 'dart:io';

import 'package:buzz/shared/contextual_agent/contextual_agent_conversation_policy.dart';
import 'package:flutter_test/flutter_test.dart';

Map<String, dynamic> loadFixture() {
  // Walk up from CWD to the monorepo root fixture.
  var dir = Directory.current;
  for (var i = 0; i < 8; i++) {
    final candidate = File(
      '${dir.path}/tests/fixtures/contextual-agent-conversation-cases.json',
    );
    if (candidate.existsSync()) {
      return jsonDecode(candidate.readAsStringSync()) as Map<String, dynamic>;
    }
    // When tests run with CWD=mobile/, also check the parent monorepo root.
    final parentCandidate = File(
      '${dir.path}/../tests/fixtures/contextual-agent-conversation-cases.json',
    );
    if (parentCandidate.existsSync()) {
      return jsonDecode(parentCandidate.readAsStringSync())
          as Map<String, dynamic>;
    }
    final parent = dir.parent;
    if (parent.path == dir.path) break;
    dir = parent;
  }
  fail(
    'could not locate tests/fixtures/contextual-agent-conversation-cases.json',
  );
}

UnaddressedChannelAgentMode parseMode(String raw) {
  switch (raw) {
    case 'mentions-only':
      return UnaddressedChannelAgentMode.mentionsOnly;
    case 'all-channel-agents':
    default:
      return UnaddressedChannelAgentMode.allChannelAgents;
  }
}

List<String> strList(Map<String, dynamic> map, String key) {
  final value = map[key];
  if (value is! List) return const [];
  return value.map((e) => e.toString()).toList();
}

String? optStr(Map<String, dynamic> map, String key) {
  final value = map[key];
  if (value == null) return null;
  return value.toString();
}

ContextualAgentConversationInput parseInput(Map<String, dynamic> input) {
  return ContextualAgentConversationInput(
    conversation: input['conversation'] as String,
    messagePosition: input['messagePosition'] as String,
    senderClass: input['senderClass'] as String,
    unaddressedMode: parseMode(input['unaddressedMode'] as String),
    keepAddressedAgentsActive: input['keepAddressedAgentsActive'] as bool,
    explicitMentionPubkeys: strList(input, 'explicitMentionPubkeys'),
    currentAgentPubkey: optStr(input, 'currentAgentPubkey'),
    channelMemberPubkeys: strList(input, 'channelMemberPubkeys'),
    verifiedChannelAgentPubkeys: strList(input, 'verifiedChannelAgentPubkeys'),
    unverifiedAgentPubkeys: strList(input, 'unverifiedAgentPubkeys'),
    nonMemberAgentPubkeys: strList(input, 'nonMemberAgentPubkeys'),
    threadRootEventId: optStr(input, 'threadRootEventId'),
    replyingUnderEventId: optStr(input, 'replyingUnderEventId'),
    persistentThreadAudience: strList(input, 'persistentThreadAudience'),
    manualRemovedPubkeys: strList(input, 'manualRemovedPubkeys'),
    recipientLoadError: input['recipientLoadError'] as bool? ?? false,
    humanMessageEventId: optStr(input, 'humanMessageEventId'),
  );
}

ReplyPlacement parseExpectedPlacement(Map<String, dynamic> expected) {
  final placement = expected['replyPlacement'] as Map<String, dynamic>;
  switch (placement['kind'] as String) {
    case 'thread-root':
      return ThreadRootPlacement(placement['eventId'] as String);
    case 'unconstrained':
      return const UnconstrainedPlacement();
    case 'top-level':
    default:
      return const TopLevelPlacement();
  }
}

void main() {
  final fixture = loadFixture();
  final cases = (fixture['cases'] as List).cast<Map<String, dynamic>>();

  test('fixture version and case count', () {
    expect(fixture['version'], 1);
    expect(cases.length, greaterThanOrEqualTo(12));
  });

  for (final c in cases) {
    final id = c['id'] as String;
    test('contextual fixture (flutter): $id', () {
      final inputMap = Map<String, dynamic>.from(c['input'] as Map);
      final expected = c['expected'] as Map<String, dynamic>;
      final placement = expected['replyPlacement'] as Map<String, dynamic>;
      if (placement['kind'] == 'thread-root' &&
          placement['eventId'] == 'human-message-id') {
        inputMap['humanMessageEventId'] = 'human-message-id';
      }
      final input = parseInput(inputMap);
      final decision = resolveContextualAgentConversation(input);

      final actualAudience = [...decision.audiencePubkeys]..sort();
      final expectedAudience = strList(expected, 'audiencePubkeys')..sort();
      expect(actualAudience, expectedAudience, reason: '$id audience');

      final expectedPlacement = parseExpectedPlacement(expected);
      expect(
        decision.replyPlacement.runtimeType,
        expectedPlacement.runtimeType,
        reason: '$id placement kind',
      );
      if (expectedPlacement is ThreadRootPlacement &&
          decision.replyPlacement is ThreadRootPlacement) {
        expect(
          (decision.replyPlacement as ThreadRootPlacement).eventId,
          expectedPlacement.eventId,
          reason: '$id thread root id',
        );
      }

      expect(
        decision.sharedThread,
        expected['sharedThread'],
        reason: '$id shared',
      );
      expect(
        decision.retainDraft,
        expected['retainDraft'],
        reason: '$id retain',
      );
      if (expected.containsKey('nestUnderAgentReply')) {
        expect(
          decision.nestUnderAgentReply,
          expected['nestUnderAgentReply'],
          reason: '$id nest',
        );
      }
    });
  }
}
