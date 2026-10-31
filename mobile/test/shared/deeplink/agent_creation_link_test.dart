import 'package:buzz/features/agents/agent_creation_draft.dart';
import 'package:buzz/shared/deeplink/deep_link.dart';
import 'package:flutter_test/flutter_test.dart';

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
          'snake_case and synonyms': (
            json: {
              'display_name': 'Scout',
              'avatar_url': avatar,
              'system_prompt': 'Be kind.',
              'model_id': 'opus-5-5',
              'reasoning_effort': 'low',
            },
            name: 'Scout',
            instructions: 'Be kind.',
            avatar: avatar,
            model: 'opus-5-5',
            effort: 'low',
          ),
          'model object': (
            json: {
              'name': 'Scout',
              'prompt': 'Hi.',
              'model': {'id': 'opus-5-5', 'effort': 'medium'},
            },
            name: 'Scout',
            instructions: 'Hi.',
            avatar: null,
            model: 'opus-5-5',
            effort: 'medium',
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
}
