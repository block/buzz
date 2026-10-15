import 'package:buzz/shared/me/me_agent_change.dart';
import 'package:buzz/shared/me/me_chat_creation.dart';
import 'package:buzz/shared/me/me_project_creation.dart';
import 'package:buzz/shared/me/me_groups.dart';
import 'package:buzz/shared/me/me_sync.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:shared_preferences/shared_preferences.dart';
import 'package:shared_preferences_platform_interface/shared_preferences_platform_interface.dart';

class _Store extends InMemorySharedPreferencesStore {
  String? failure;
  _Store() : super.empty();
  @override
  Future<bool> remove(String key) async {
    if (failure == 'throw') throw StateError('disk failure');
    if (failure == 'false') return false;
    return super.remove(key);
  }
}

class _Sync extends MeSync {
  MeGroups groups = const MeGroups();
  _Sync(SharedPreferences prefs)
    : super(
        RelaySessionNotifier(),
        RelayConfig(
          baseUrl: 'https://relay.test',
          nsec: nostr.Nip19.encode(
            prefix: nostr.Nip19Prefix.nsec,
            data: '0' * 63 + '1',
          ),
        ),
        prefs: prefs,
      );
  @override
  Future<NostrEvent?> head() async => null;
  @override
  Future<void> update(MeGroups Function(MeGroups) change) async {
    groups = change(groups);
  }
}

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  for (final kind in ['chat', 'project', 'agent']) {
    for (final action in ['complete', 'discard']) {
      for (final failure in ['false', 'throw']) {
        test(
          '$kind $action retains same-process and restarted recovery after $failure removal',
          () async {
            SharedPreferences.setMockInitialValues({});
            final store = _Store();
            SharedPreferencesStorePlatform.instance = store;
            final prefs = await SharedPreferences.getInstance();
            final sync = _Sync(prefs);
            const chat = MeChatCreation('chat-id', 'Chat', 'agent');
            const project = MeProjectCreation('project-id', 'Project');
            final agent = MeAgentChange.create(sync, 'chat-id', 'agent');
            var created = false, member = false, placed = false;
            var creates = 0, adds = 0, placements = 0;
            Future<void> save() async {
              if (kind == 'chat') await chat.save(sync);
              if (kind == 'project') await project.save(sync);
              if (kind == 'agent') await agent.save(sync, 'chat-id');
            }

            Object? pending() => switch (kind) {
              'chat' => MeChatCreation.pending(sync)?.id,
              'project' => MeProjectCreation.pending(sync)?.id,
              _ => MeAgentChange.pending(sync, 'chat-id')?.agent,
            };
            Future<void> run() async {
              if (action == 'discard') {
                if (kind == 'chat') await chat.discard(sync);
                if (kind == 'project') await project.discard(sync);
                if (kind == 'agent') await agent.discard(sync, 'chat-id');
              } else if (kind == 'chat') {
                await chat.complete(
                  sync,
                  findChannel: () async => created ? 'chat-id' : null,
                  createChannel: () async {
                    creates++;
                    created = true;
                    return 'chat-id';
                  },
                  hasAgent: () async => member,
                  addAgent: () async {
                    adds++;
                    member = true;
                  },
                  isPlaced: () async => placed,
                  place: () async {
                    placements++;
                    placed = true;
                  },
                );
              } else if (kind == 'project') {
                await project.complete(sync);
              } else {
                await agent.complete(
                  sync,
                  'chat-id',
                  ensureMembership: () async {
                    if (!member) {
                      adds++;
                      member = true;
                    }
                  },
                );
              }
            }

            await save();
            final original = pending();
            store.failure = failure;
            await expectLater(run(), throwsStateError);
            expect(
              pending(),
              original,
            ); // Reopening without restarting recovers.
            await prefs.reload();
            expect(pending(), original); // Disk agrees after restart.
            store.failure = null;
            await run();
            expect(pending(), isNull);
            if (action == 'complete') {
              if (kind == 'chat') {
                expect([creates, adds, placements], [1, 1, 1]);
              }
              if (kind == 'project') expect(sync.groups.groups.length, 1);
              if (kind == 'agent') expect(adds, 1);
            }
          },
        );
      }
    }
  }

  test('chat completion preserves a replacement journal', () async {
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();
    final sync = _Sync(prefs);
    const intent = MeChatCreation('first', 'First', 'agent');
    await expectLater(
      intent.complete(
        sync,
        findChannel: () async => 'first',
        createChannel: () async => 'first',
        hasAgent: () async => true,
        addAgent: () async {},
        isPlaced: () async => false,
        place: () async {
          await prefs.remove(MeChatCreation.storageKey(sync));
          await const MeChatCreation('second', 'Second', 'agent').save(sync);
        },
      ),
      throwsStateError,
    );
    expect(MeChatCreation.pending(sync)?.id, 'second');
  });
}
