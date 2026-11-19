import 'package:flutter_test/flutter_test.dart';
import 'package:buzz/features/channels/agent_activity/work_snapshot.dart';

void main() {
  test('retiring the newer turn reveals the still-active older turn', () {
    WorkSnapshot frame(String turn, int start, int update, bool active) =>
        WorkSnapshot(
          turnId: turn,
          threadRootId: 'thread',
          startedAt: start,
          updatedAt: update,
          active: active,
          steps: [turn],
        );
    final frames = [
      frame('older', 100, 1000, true),
      frame('newer', 200, 1000, true),
      frame('newer', 200, 1001, false),
    ];
    expect(currentWorkSnapshot(frames, {'thread'}, 1002)?.turnId, 'older');
    expect(currentWorkSnapshot(frames, {'other'}, 1002), isNull);
    expect(currentWorkSnapshot(frames, {'thread'}, 17000), isNull);
  });
}
