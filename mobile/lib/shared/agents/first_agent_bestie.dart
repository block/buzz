import '../me/default_agent_writer.dart';
import '../me/me_sync.dart';
import 'owned_agent_directory.dart';

/// Makes the newly created agent Bestie only when it is the owner's first.
/// Fresh relay reads avoid treating a loading directory as an empty one.
Future<void> makeFirstAgentBestie(MeSync sync, String pubkey) async {
  sync.check();
  final owned = await loadOwnedAgentDirectory(
    sync.pubkey,
    sync.session.fetchHistory,
  );
  sync.check();
  if (!owned.any((agent) => agent.pubkey == pubkey)) {
    throw StateError(
      'Agent registration is still syncing. Resume setup to retry.',
    );
  }
  if (owned.length == 1) {
    await saveDefaultAgent(sync, pubkey, onlyIfUnset: true);
  }
}
