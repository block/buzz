import 'dart:convert';
import 'package:flutter_test/flutter_test.dart';
import 'package:buzz/features/agents/community_bot_profiles.dart';
import 'package:buzz/shared/relay/relay.dart';

void main() {
  test('only kind-0 profiles with explicit boolean bot status are agents', () {
    for (final entry in [
      (0, jsonEncode({'bot': true, 'name': 'Swiss'}), true),
      (0, jsonEncode({'bot': false}), false),
      (0, jsonEncode({'bot': 'true'}), false),
      (0, jsonEncode({'name': 'Person'}), false),
      (0, '[]', false),
      (0, 'invalid', false),
      (10100, jsonEncode({'bot': true}), false),
    ]) {
      final event = NostrEvent.fromJson({
        'id': 'a' * 64,
        'pubkey': 'b' * 64,
        'created_at': 1,
        'kind': entry.$1,
        'tags': <List<String>>[],
        'content': entry.$2,
        'sig': '0' * 128,
      });
      expect(isCommunityBotProfile(event), entry.$3);
    }
  });
}
