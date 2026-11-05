import 'dart:convert';

import 'package:buzz/features/agents/agent_creation_draft.dart';
import 'package:buzz/shared/deeplink/deep_link.dart';
import 'package:buzz/shared/hosted/hosted_environment.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';

void main() {
  group('buzz://agent/new links', () {
    const accepted = {
      'card id': (
        'buzz://agent/new?card=av_MioTAG6dm7q3Zcm0UV2j8A',
        'av_MioTAG6dm7q3Zcm0UV2j8A',
      ),
      'any opaque id': (
        'buzz://agent/new?card=card.7~x%2Fy%20z',
        'card.7~x/y z',
      ),
    };
    accepted.forEach((label, c) {
      test(label, () {
        expect(parseBuzzDeepLink(Uri.parse(c.$1)), AgentCreationDeepLink(c.$2));
      });
    });

    const rejected = {
      'no card': 'buzz://agent/new',
      'empty card': 'buzz://agent/new?card=',
      'dot segment': 'buzz://agent/new?card=..',
      'two cards': 'buzz://agent/new?card=a&card=b',
      'extra parameter': 'buzz://agent/new?card=a&auto=true',
      'inline draft (retired format)': 'buzz://agent/new#e30',
      'card plus fragment': 'buzz://agent/new?card=a#e30',
      'wrong path': 'buzz://agent/other?card=a',
      'wrong host': 'buzz://agents/new?card=a',
      'https share page': 'https://app.builderlab.xyz/a/av_x',
    };
    rejected.forEach((label, uri) {
      test(label, () {
        expect(
          parseBuzzDeepLink(Uri.parse(uri)),
          isNot(isA<AgentCreationDeepLink>()),
        );
      });
    });
  });

  group('agent.json', () {
    const avatar = 'https://cdn.example/av_1/rev/avatar.png';
    // A card as kgoose publishes it today (schema v1).
    const published = <String, dynamic>{
      'v': 1,
      'id': 'av_1',
      'collection': 'gloopie',
      'name': 'Nitrox',
      'number': 100,
      'avatarUrl': avatar,
      'cardUrl': 'https://cdn.example/av_1/rev/card.png',
      'shareUrl': 'https://app.builderlab.xyz/a/av_1',
      'description': 'Deep-water scout.',
    };

    test('a published card opens with name and avatar only', () {
      final draft = AgentCreationDraft.parseAgentCard(published, 'av_1');
      expect(draft.name, 'Nitrox');
      expect(draft.instructions, '');
      expect(draft.avatar?.url, avatar);
      expect(draft.modelId, isNull);
      expect(draft.reasoningEffort, isNull);
    });

    test('a card carries instructions and model defaults', () {
      final draft = AgentCreationDraft.parseAgentCard({
        ...published,
        'v': 99,
        'name': ' Scout ',
        'avatarUrl': 'https://images.example/scout.png',
        'instructions': 'Reply concisely.',
        'model': 'sonnet-5-5',
        'reasoningEffort': 'high',
        // Fields Buzz doesn't know are ignored whatever their type, so
        // Builderlab can add them without breaking older apps.
        'description': {'unexpected': 'shape'},
        'tags': ['ocean', 7],
        'rarity': null,
        'nested': {
          'deep': [true, 1.5],
        },
        'system_prompt': 'ignored',
        'model_id': 'ignored',
      }, 'av_1');
      expect(draft.name, 'Scout');
      expect(draft.avatar?.url, 'https://images.example/scout.png');
      expect(draft.instructions, 'Reply concisely.');
      expect(draft.modelId, 'sonnet-5-5');
      expect(draft.reasoningEffort, 'high');
    });

    test('blank optional fields are left for the user', () {
      final draft = AgentCreationDraft.parseAgentCard({
        ...published,
        'instructions': '',
        'model': ' ',
      }, 'av_1');
      expect(draft.instructions, '');
      expect(draft.modelId, isNull);
    });

    final invalid = <String, Map<String, dynamic>>{
      'missing id': {...published}..remove('id'),
      'id of another card': {...published, 'id': 'av_2'},
      'missing name': {...published}..remove('name'),
      'blank name': {...published, 'name': ' '},
      'non-string name': {...published, 'name': 7},
      'missing avatar': {...published}..remove('avatarUrl'),
      'non-string avatar': {...published, 'avatarUrl': 7},
      'http avatar': {...published, 'avatarUrl': 'http://cdn.example/a.png'},
      'relative avatar': {...published, 'avatarUrl': '/av_1/a.png'},
      'non-string instructions': {
        ...published,
        'instructions': ['x'],
      },
      'non-string model': {...published, 'model': 5},
      'non-string effort': {
        ...published,
        'model': 'sonnet-5-5',
        'reasoningEffort': true,
      },
    };
    invalid.forEach((label, json) {
      test(label, () {
        expect(
          () => AgentCreationDraft.parseAgentCard(json, 'av_1'),
          throwsA(
            isA<AgentCardException>().having(
              (e) => e.message,
              'message',
              'This agent card is invalid.',
            ),
          ),
        );
      });
    });
  });

  group('fetching agent.json', () {
    const link = AgentCreationDeepLink('av_1');
    final big = 'x' * (maxAgentCardBytes + 1);

    Future<Object> outcome(MockClientStreamHandler handler) async {
      try {
        return await AgentCreationDraft.fetch(
          link,
          MockClient.streaming(handler),
        );
      } on AgentCardException catch (error) {
        return error.message;
      }
    }

    http.StreamedResponse respond(
      int status, [
      List<List<int>> chunks = const [],
      int? length,
    ]) => http.StreamedResponse(
      Stream.fromIterable(chunks),
      status,
      contentLength: length,
    );

    test('requests the build CDN without following redirects', () async {
      late http.BaseRequest seen;
      final result = await outcome((request, _) async {
        seen = request;
        return respond(200, [
          utf8.encode(
            jsonEncode({
              'v': 1,
              'id': 'av_1',
              'name': 'Scout',
              'avatarUrl': HostedEnvironment.avatarsCdnOrigin
                  .replace(path: '/av_1/r/avatar.png')
                  .toString(),
            }),
          ),
        ]);
      });
      expect(result, isA<AgentCreationDraft>());
      expect(
        seen.url,
        HostedEnvironment.avatarsCdnOrigin.replace(path: '/av_1/agent.json'),
      );
      expect(seen.followRedirects, isFalse);
    });

    test('percent-encodes the opaque id as one path segment', () async {
      late Uri seen;
      await AgentCreationDraft.fetch(
        const AgentCreationDeepLink('a/b c'),
        MockClient.streaming((request, _) async {
          seen = request.url;
          return respond(404);
        }),
      ).then<void>((_) {}, onError: (_) {});
      expect(seen.path, '/a%2Fb%20c/agent.json');
    });

    final failures = <String, (MockClientStreamHandler, String)>{
      '403 from S3': (
        (_, _) async => respond(403),
        'This agent card isn’t available.',
      ),
      '404': ((_, _) async => respond(404), 'This agent card isn’t available.'),
      'redirect': (
        (_, _) async => respond(302),
        'Couldn’t load this agent card. Try again later.',
      ),
      '500': (
        (_, _) async => respond(500),
        'Couldn’t load this agent card. Try again later.',
      ),
      'network error': (
        (_, _) async => throw http.ClientException('offline'),
        'Couldn’t load this agent card. Check your connection and reopen the '
            'link.',
      ),
      'declared length over the cap': (
        (_, _) async => respond(200, const [], maxAgentCardBytes + 1),
        'This agent card is too large.',
      ),
      'streamed body over the cap': (
        (_, _) async =>
            respond(200, [utf8.encode('{"name":"'), utf8.encode(big)]),
        'This agent card is too large.',
      ),
      'not JSON': (
        (_, _) async => respond(200, [utf8.encode('<html>')]),
        'This agent card is invalid.',
      ),
      'JSON array': (
        (_, _) async => respond(200, [utf8.encode('[]')]),
        'This agent card is invalid.',
      ),
    };
    failures.forEach((label, c) {
      test(label, () async => expect(await outcome(c.$1), c.$2));
    });
  });
}
