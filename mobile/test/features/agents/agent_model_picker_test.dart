import 'package:buzz/features/agents/agent_model_picker.dart';
import 'package:buzz/shared/agents/remote_agents.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  const opus = RemoteAgentModel(
    id: 'kgoose-claude-opus-4-6',
    name: 'Opus',
    efforts: ['low', 'medium', 'high'],
  );
  const sonnet = RemoteAgentModel(
    id: 'kgoose-claude-sonnet-4-6',
    name: 'Sonnet',
    efforts: ['low', 'medium', 'high'],
  );

  test('defaults to Sonnet at medium effort', () {
    expect(startingAgentModel([opus, sonnet]), (
      'kgoose-claude-sonnet-4-6',
      'medium',
    ));
  });

  test('leaves effort for the user when medium is not offered', () {
    const highOnly = RemoteAgentModel(
      id: 'claude-sonnet-5',
      name: 'Sonnet 5',
      efforts: ['high'],
    );
    expect(startingAgentModel([opus, highOnly]), ('claude-sonnet-5', null));
  });

  test('chooses nothing when no Sonnet is offered', () {
    expect(startingAgentModel([opus]), (null, null));
  });

  test('a draft model the catalog offers wins over the default', () {
    expect(
      startingAgentModel(
        [opus, sonnet],
        draftModel: opus.id,
        draftEffort: 'high',
      ),
      (opus.id, 'high'),
    );
    expect(
      startingAgentModel(
        [opus, sonnet],
        draftModel: opus.id,
        draftEffort: 'max',
      ),
      (opus.id, null),
    );
  });

  test('a draft model the catalog lacks falls back to the default', () {
    expect(
      startingAgentModel(
        [opus, sonnet],
        draftModel: 'gone',
        draftEffort: 'high',
      ),
      (sonnet.id, 'medium'),
    );
  });
}
