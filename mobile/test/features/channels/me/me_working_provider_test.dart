import 'package:buzz/features/channels/channel_typing_provider.dart';
import 'package:buzz/features/channels/me/me_working_provider.dart';
import 'package:buzz/shared/identity_names/identity_names.dart';
import 'package:buzz/shared/identity_names/identity_names_provider.dart';
import 'package:buzz/shared/me/me_assistant_names.dart';
import 'package:buzz/shared/me/me_assistants.dart';
import 'package:buzz/shared/mentions/agent_identity_provider.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

class _Typing extends ChannelTypingNotifier {
  _Typing(super.channelId);
  @override
  List<TypingEntry> build() => [];
  void show(List<String> pubkeys) => state = [
    for (final pubkey in pubkeys)
      TypingEntry(pubkey: pubkey, expiresAtMs: 999999),
  ];
}

void main() {
  test('idle Solo rows build no names; working namesakes read apart', () async {
    final first = 'a1' * 32;
    final second = 'b2' * 32;
    final chats = ['one', 'two'];
    final typing = {for (final id in chats) id: _Typing(id)};
    var nameReads = 0;
    final container = ProviderContainer(
      overrides: [
        meAssistantsProvider.overrideWith(
          (ref) async => [
            for (final key in [first, second])
              AgentDirectoryEntry(pubkey: key, displayName: 'Atlas'),
          ],
        ),
        identityNameSourcesProvider.overrideWith((ref) {
          nameReads++;
          return IdentityNameSources.empty;
        }),
        for (final id in chats)
          channelTypingProvider(id).overrideWith(() => typing[id]!),
      ],
    );
    addTearDown(container.dispose);
    await container.read(meAssistantsProvider.future);
    final rows = {
      for (final id in chats)
        id: container.listen(meWorkingAgentsProvider(id), (_, _) {}),
    };

    // Nobody is working: no row copies the profile cache into a name context.
    expect([for (final row in rows.values) row.read()], [isEmpty, isEmpty]);
    expect(nameReads, 0);
    expect(container.exists(meAssistantNamesProvider), isFalse);

    typing['one']!.show([first]);
    typing['two']!.show([first, second]);
    final one = rows['one']!.read();
    final two = rows['two']!.read();
    expect(one, hasLength(1));
    expect(two, hasLength(2));
    expect(two.toSet(), hasLength(2), reason: 'Namesakes must differ');
    expect(two.first, one.single, reason: 'One agent reads alike everywhere');
    expect(two, everyElement(startsWith('Atlas')));
  });
}
