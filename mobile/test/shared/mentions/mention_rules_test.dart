import 'dart:convert';
import 'dart:io';

import 'package:buzz/shared/mentions/mention_rules.dart';
import 'package:buzz/shared/mentions/mention_tags.dart';
import 'package:flutter_test/flutter_test.dart';

// Portable fixtures, copied unchanged from buzz-app
// src/bundled/mentions/mention-rules.fixtures.json (mention rules v1).
final Map<String, dynamic> _fixtures =
    jsonDecode(
          File(
            'test/shared/mentions/mention-rules.fixtures.json',
          ).readAsStringSync(),
        )
        as Map<String, dynamic>;

List<Map<String, dynamic>> _cases(String section) => [
  for (final item in _fixtures[section] as List) item as Map<String, dynamic>,
];

List<Map<String, dynamic>> _tagCases(String section) => [
  for (final item in (_fixtures['tags'] as Map)[section] as List)
    item as Map<String, dynamic>,
];

MentionChoice _choice(Map<String, dynamic> json) {
  final name = json['name'] as String;
  return MentionChoice(
    pubkey: json['pubkey'] as String,
    name: name,
    label: json['label'] as String? ?? name,
    aliases: [
      for (final alias in json['aliases'] as List? ?? [name]) alias as String,
    ],
    member: json['member'] as bool,
    agent: json['agent'] as bool? ?? false,
    owned: json['owned'] as bool? ?? false,
    managed: json['managed'] as bool? ?? false,
  );
}

List<MentionChoice> _choices(Map<String, dynamic> fixture) => [
  for (final choice in fixture['choices'] as List)
    _choice(choice as Map<String, dynamic>),
];

void main() {
  test('reads mention fixtures version 1', () {
    expect(_fixtures['version'], 1);
    for (final section in ['ranking', 'space', 'query', 'admission']) {
      expect(_cases(section), isNotEmpty, reason: section);
    }
    expect(_tagCases('write'), isNotEmpty);
    expect(_tagCases('read'), isNotEmpty);
  });

  group('tag writer fixtures', () {
    for (final fixture in _tagCases('write')) {
      test(fixture['name'], () {
        List<String> keys(String field) => [
          for (final key in fixture[field] as List) key as String,
        ];
        final expected = fixture['expected'] as Map;
        // The sender is a member in addition to `members`.
        final members = {...keys('members'), 'f' * 64};
        Object? written() {
          try {
            return {
              'tags': writeMentionTags(
                recipients: keys('recipients'),
                references: keys('references'),
                members: members,
              ),
            };
          } on MentionTagException catch (error) {
            return {
              'error': switch (error.error) {
                MentionTagError.notMember => 'not_member',
                MentionTagError.invalid => 'invalid',
                MentionTagError.tooMany => 'too_many',
              },
            };
          }
        }

        expect(written(), expected);
      });
    }
  });

  group('tag reader fixtures', () {
    for (final fixture in _tagCases('read')) {
      test(fixture['name'], () {
        final read = readMentionTags([
          for (final tag in fixture['tags'] as List)
            [for (final field in tag as List) field as String],
        ]);
        expect({
          'mentions': read.mentions,
          'references': read.references,
        }, fixture['expected']);
      });
    }
  });

  group('ranking fixtures', () {
    for (final fixture in _cases('ranking')) {
      test(fixture['name'], () {
        final history = {
          for (final entry in (fixture['history'] as Map? ?? const {}).entries)
            entry.key as String: entry.value as int,
        };
        final presence = fixture['presence'] as Map? ?? const {};
        final ranked = rankMentions(
          _choices(fixture),
          fixture['query'] as String,
          history: history,
          presence: (key) => presence[key] as String? ?? 'unknown',
        );
        expect([for (final c in ranked) c.pubkey], fixture['expected']);
      });
    }
  });

  group('Space fixtures', () {
    for (final fixture in _cases('space')) {
      test(fixture['name'], () {
        expect(
          exactMention(_choices(fixture), fixture['query'] as String),
          fixture['expected'],
        );
      });
    }
  });

  group('query syntax fixtures', () {
    for (final fixture in _cases('query')) {
      test(fixture['name'], () {
        final query = findMentionQuery(
          fixture['text'] as String,
          fixture['caret'] as int,
        );
        final expected = fixture['expected'] as Map?;
        expect(
          query == null
              ? null
              : {'start': query.start, 'end': query.end, 'query': query.query},
          expected,
        );
      });
    }
  });

  group('multi-word admission fixtures', () {
    for (final fixture in _cases('admission')) {
      test(fixture['name'], () {
        expect(
          matchesMentionQuery(fixture['query'] as String, [
            for (final name in fixture['names'] as List) name as String,
          ]),
          fixture['expected'],
        );
      });
    }
  });

  test('order does not depend on input order', () {
    // Review case: owned Zed, unowned Alpha, human Mary.
    const zed = MentionChoice(
      pubkey: 'a',
      name: 'Zed',
      label: 'Zed',
      aliases: ['Zed'],
      member: true,
      agent: true,
      owned: true,
    );
    const alpha = MentionChoice(
      pubkey: 'b',
      name: 'Alpha',
      label: 'Alpha',
      aliases: ['Alpha'],
      member: true,
      agent: true,
    );
    const mary = MentionChoice(
      pubkey: 'c',
      name: 'Mary',
      label: 'Mary',
      aliases: ['Mary'],
      member: true,
    );
    for (final rows in [
      [zed, alpha, mary],
      [zed, mary, alpha],
      [alpha, zed, mary],
      [alpha, mary, zed],
      [mary, zed, alpha],
      [mary, alpha, zed],
    ]) {
      expect(rankMentions(rows, ''), [zed, alpha, mary]);
    }
  });
}
