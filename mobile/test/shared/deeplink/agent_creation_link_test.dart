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
      'any URL-safe token': (
        'buzz://agent/new?card=abc-DEF_123',
        'abc-DEF_123',
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
      'path in card': 'buzz://agent/new?card=..%2Fx',
      'host in card': 'buzz://agent/new?card=evil.example%2Fx',
      'card too long':
          'buzz://agent/new?card=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
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
    final cdn = Uri.parse('https://cdn.example');
    const avatar = 'https://cdn.example/av_1/rev/avatar.png';
    final cases =
        <
          String,
          ({
            Map<String, dynamic> json,
            String name,
            String instructions,
            String? avatar,
            String? model,
            String? effort,
          })
        >{
          'published v1 card: name and avatar only': (
            json: {
              'v': 1,
              'id': 'av_1',
              'collection': 'gloopie',
              'name': 'Nitrox',
              'number': 100,
              'avatarUrl': avatar,
              'cardUrl': 'https://cdn.example/av_1/rev/card.png',
              'shareUrl': 'https://app.builderlab.xyz/a/av_1',
            },
            name: 'Nitrox',
            instructions: '',
            avatar: avatar,
            model: null,
            effort: null,
          ),
          'canonical keys': (
            json: {
              'name': ' Scout ',
              'avatarUrl': avatar,
              'instructions': 'Reply concisely.',
              'model': 'sonnet-5-5',
              'reasoningEffort': 'high',
            },
            name: 'Scout',
            instructions: 'Reply concisely.',
            avatar: avatar,
            model: 'sonnet-5-5',
            effort: 'high',
          ),
          'other key names are ignored': (
            json: {
              'name': 'Scout',
              'avatar_url': avatar,
              'system_prompt': 'Be kind.',
              'model_id': 'opus-5-5',
              'reasoning_effort': 'low',
            },
            name: 'Scout',
            instructions: '',
            avatar: null,
            model: null,
            effort: null,
          ),
          'wrong types and blanks are left for the user': (
            json: {
              'name': 7,
              'instructions': ['x'],
              'model': '  ',
              'reasoningEffort': true,
            },
            name: '',
            instructions: '',
            avatar: null,
            model: null,
            effort: null,
          ),
          'avatar on another host is not used': (
            json: {'name': 'Scout', 'avatarUrl': 'https://evil.example/a.png'},
            name: 'Scout',
            instructions: '',
            avatar: null,
            model: null,
            effort: null,
          ),
          'http avatar is not used': (
            json: {'name': 'Scout', 'avatarUrl': 'http://cdn.example/a.png'},
            name: 'Scout',
            instructions: '',
            avatar: null,
            model: null,
            effort: null,
          ),
          'avatar with credentials is not used': (
            json: {
              'name': 'Scout',
              'avatarUrl': 'https://u:p@cdn.example/a.png',
            },
            name: 'Scout',
            instructions: '',
            avatar: null,
            model: null,
            effort: null,
          ),
        };
    cases.forEach((label, c) {
      test(label, () {
        final draft = AgentCreationDraft.parseAgentCard(c.json, cdn);
        expect(draft.name, c.name);
        expect(draft.instructions, c.instructions);
        expect(draft.avatar?.url, c.avatar);
        expect(draft.modelId, c.model);
        expect(draft.reasoningEffort, c.effort);
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
        return respond(200, [utf8.encode('{"name":"Scout"}')]);
      });
      expect(result, isA<AgentCreationDraft>());
      expect(
        seen.url,
        HostedEnvironment.avatarsCdnOrigin.replace(path: '/av_1/agent.json'),
      );
      expect(seen.followRedirects, isFalse);
    });

    final failures = <String, (MockClientStreamHandler, String)>{
      '403 from S3': (
        (_, _) async => respond(403),
        'This agent card isn’t available.',
      ),
      '404': ((_, _) async => respond(404), 'This agent card isn’t available.'),
      'redirect': (
        (_, _) async => respond(302),
        'Couldn’t load this agent card (HTTP 302).',
      ),
      '500': (
        (_, _) async => respond(500),
        'Couldn’t load this agent card (HTTP 500).',
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
