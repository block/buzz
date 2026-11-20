import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channel_sections/filtered_channel_section_body.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  testWidgets('disclosure keeps hundreds of filtered rows lazy', (
    tester,
  ) async {
    final channels = [
      for (var index = 0; index < 300; index++)
        Channel(
          id: 'chat-$index',
          name: 'Chat $index',
          channelType: 'stream',
          visibility: 'open',
          description: '',
          memberCount: 1,
          createdBy: 'owner',
          createdAt: DateTime(2026),
          isMember: true,
        ),
    ];
    final built = <String>{};
    await tester.pumpWidget(
      MaterialApp(
        home: Scaffold(
          body: CustomScrollView(
            slivers: [
              FilteredChannelSectionBody(
                expanded: true,
                channels: channels,
                isVisible: (_) => false,
                buildRow: (channel) {
                  built.add(channel.id);
                  return SizedBox(
                    key: ValueKey(channel.id),
                    height: 44,
                    child: Text(channel.name),
                  );
                },
              ),
            ],
          ),
        ),
      ),
    );
    expect(built, isEmpty);

    await tester.tap(find.text('Show more'));
    await tester.pump();
    expect(built, isNotEmpty);
    expect(built.length, lessThan(60));
    await tester.pumpAndSettle();
    expect(find.text('Chat 0'), findsOneWidget);
    expect(find.text('Chat 299'), findsNothing);

    await tester.scrollUntilVisible(find.text('Show fewer'), 500);
    expect(find.text('Chat 299'), findsOneWidget);
    await tester.tap(find.text('Show fewer'));
    await tester.pumpAndSettle();
    expect(find.text('Chat 299'), findsNothing);
    expect(find.text('Show more'), findsOneWidget);
  });
}
