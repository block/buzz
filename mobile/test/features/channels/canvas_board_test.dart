import 'package:buzz/features/channels/canvas_board.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  test('parses durable card metadata and hides it from the body', () {
    final threadId = 'b' * 64;
    final board = parseCanvasBoard(
      '# Dispatch\n\nA shared work surface.\n\n'
      '## Decide the next move\n\n'
      '<!-- buzz-board-card {"id":"decision-1","type":"decision","status":"doing","thread":"$threadId","author":"alice"} -->\n\n'
      'Choose the smallest complete loop.',
    );

    expect(board.title, 'Dispatch');
    expect(board.introduction, 'A shared work surface.');
    expect(board.cards, hasLength(1));
    final card = board.cards.single;
    expect(card.id, 'decision-1');
    expect(card.type, CanvasBoardCardType.decision);
    expect(card.status, CanvasBoardCardStatus.doing);
    expect(card.threadId, threadId);
    expect(card.authorPubkey, 'alice');
    expect(card.body, 'Choose the smallest complete loop.');
  });

  test('leaves headings inside fenced Markdown in the card body', () {
    final board = parseCanvasBoard(
      '## Notes\n\n```md\n## Not a card\n```\n\n## Next action\n\nShip it.',
    );

    expect(board.cards, hasLength(2));
    expect(board.cards.first.body, contains('## Not a card'));
    expect(board.cards.last.type, CanvasBoardCardType.task);
  });

  test('keeps Dispatch board-first and excludes DMs', () {
    expect(
      channelHasCanvasBoard(
        channelName: '#Dispatch',
        isDm: false,
        content: null,
      ),
      isTrue,
    );
    expect(
      channelHasCanvasBoard(
        channelName: 'Dispatch',
        isDm: true,
        content: '# Hidden',
      ),
      isFalse,
    );
    expect(
      initialChannelCanvasView(channelName: 'Dispatch', storedValue: null),
      ChannelCanvasView.board,
    );
    expect(
      initialChannelCanvasView(
        channelName: 'Dispatch',
        storedValue: ChannelCanvasView.stream.name,
      ),
      ChannelCanvasView.stream,
    );
  });
}
