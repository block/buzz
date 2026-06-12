import 'package:flutter_test/flutter_test.dart';
import 'package:buzz/features/channels/agent_activity/observer_models.dart';
import 'package:buzz/features/channels/agent_activity/transcript_builder.dart';

void main() {
  test('aggregates assistant chunks until another item seals the message', () {
    final items = buildTranscript([
      _updateFrame(
        seq: 1,
        update: {
          'sessionUpdate': 'agent_message_chunk',
          'messageId': 'm1',
          'content': [
            {'type': 'text', 'text': 'Hello'},
          ],
        },
      ),
      _updateFrame(
        seq: 2,
        update: {
          'sessionUpdate': 'agent_message_chunk',
          'messageId': 'm1',
          'content': [
            {'type': 'text', 'text': ' world'},
          ],
        },
      ),
      _updateFrame(
        seq: 3,
        update: {
          'sessionUpdate': 'tool_call',
          'toolCallId': 'tool-1',
          'title': 'sleep',
          'args': {'seconds': 5},
        },
      ),
      _updateFrame(
        seq: 4,
        update: {
          'sessionUpdate': 'agent_message_chunk',
          'messageId': 'm1',
          'content': [
            {'type': 'text', 'text': 'Done'},
          ],
        },
      ),
    ]);

    expect(items, hasLength(3));
    expect(items[0], isA<MessageItem>());
    expect((items[0] as MessageItem).text, 'Hello world');
    expect(items[1], isA<ToolItem>());
    expect(items[2], isA<MessageItem>());
    expect((items[2] as MessageItem).text, 'Done');
  });

  test('normalizes buzz tool calls and applies result updates', () {
    final items = buildTranscript([
      _updateFrame(
        seq: 1,
        update: {
          'sessionUpdate': 'tool_call',
          'toolCallId': 'send-1',
          'title': 'Sending message to channel',
          'status': 'executing',
          'args': {'content': 'hi'},
        },
      ),
      _updateFrame(
        seq: 2,
        update: {
          'sessionUpdate': 'tool_call_update',
          'toolCallId': 'send-1',
          'status': 'completed',
          'content': [
            {'type': 'text', 'text': 'posted #activity-test-channel'},
          ],
        },
      ),
    ]);

    expect(items, hasLength(1));
    expect(items.single, isA<ToolItem>());
    final tool = items.single as ToolItem;
    expect(tool.buzzToolName, 'send_message');
    expect(tool.toolName, 'send_message');
    expect(tool.status, ToolStatus.completed);
    expect(tool.args, {'content': 'hi'});
    expect(tool.result, 'posted #activity-test-channel');
  });

  test('parses buzz prompt text into user message and metadata', () {
    final items = buildTranscript([
      ObserverFrame(
        seq: 1,
        timestamp: _timestamp(1),
        kind: 'acp_write',
        turnId: 'turn-1',
        payload: {
          'method': 'session/prompt',
          'params': {
            'prompt': [
              {
                'content':
                    '[Buzz event: stream message]\n'
                    'Content: @claude can you do that again?\n\n'
                    '[Channel]\n'
                    '#activity-test-channel',
              },
            ],
          },
        },
      ),
    ]);

    expect(items, hasLength(2));
    expect(items[0], isA<MessageItem>());
    final message = items[0] as MessageItem;
    expect(message.role, 'user');
    expect(message.title, 'Stream Message');
    expect(message.text, '@claude can you do that again?');
    expect(items[1], isA<MetadataItem>());
    expect((items[1] as MetadataItem).sections, hasLength(2));
  });
}

ObserverFrame _updateFrame({
  required int seq,
  required Map<String, dynamic> update,
}) {
  return ObserverFrame(
    seq: seq,
    timestamp: _timestamp(seq),
    kind: 'acp_read',
    turnId: 'turn-1',
    payload: {
      'method': 'session/update',
      'params': {'update': update},
    },
  );
}

String _timestamp(int seq) =>
    DateTime.utc(2026, 4, 30, 12, 0, seq).toIso8601String();
