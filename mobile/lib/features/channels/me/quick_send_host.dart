part of 'me_page.dart';

const _quickSendChannel = MethodChannel('buzz/quick_send');

/// A quick send Buzz turned down, with a reason Siri reads out.
class _QuickSendRefused implements Exception {
  const _QuickSendRefused(this.message);
  final String message;
}

/// Answers quick sends from Siri and Shortcuts while Home is mounted.
///
/// iOS can run a quick send with Buzz in the background; the app still
/// builds Home then, so this leaf is where the send gets the community's
/// session and Bestie. It tells the platform once it can take sends.
class QuickSendHost extends HookConsumerWidget {
  const QuickSendHost({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    // Keeps the Bestie choice loaded for a send that arrives in the background.
    ref.watch(defaultAgentProvider);
    useEffect(() {
      _quickSendChannel.setMethodCallHandler((call) async {
        try {
          return switch ((call.method, call.arguments)) {
            ('bestie', _) => await _bestie(ref),
            ('send', {'text': final String text, 'bestie': final String to}) =>
              await _quickSend(ref, text.trim(), to),
            _ => throw MissingPluginException(),
          };
        } on _QuickSendRefused catch (failure) {
          throw PlatformException(code: 'failed', message: failure.message);
        } on MissingPluginException {
          rethrow;
        } catch (failure) {
          debugPrint('Quick send failed: $failure');
          throw PlatformException(
            code: 'failed',
            message: "Buzz couldn't send that. Open Buzz to try again.",
          );
        }
      });
      unawaited(_quickSendChannel.invokeMethod<void>('ready'));
      return () {
        _quickSendChannel.setMethodCallHandler(null);
        unawaited(_quickSendChannel.invokeMethod<void>('unready'));
      };
    }, const []);
    return const SizedBox.shrink();
  }
}

/// The Bestie a quick send will go to, named so the person can confirm it.
///
/// This is the saved choice, which may predate a change made on another
/// device that this one hasn't synced; naming it lets the person catch that.
Future<Map<String, String?>> _bestie(WidgetRef ref) async {
  final bestie = await ref
      .read(defaultAgentProvider.future)
      .timeout(const Duration(seconds: 10));
  if (bestie == null) throw _QuickSendRefused('Choose a Bestie in Buzz first.');
  if (ref.read(userCacheProvider)[bestie] == null) {
    // Briefly: Siri can say "your Bestie" rather than keep the person waiting.
    await ref
        .read(userCacheProvider.notifier)
        .preload([bestie])
        .timeout(const Duration(seconds: 2), onTimeout: () => false);
  }
  return {'pubkey': bestie, 'name': ref.read(userCacheProvider)[bestie]?.label};
}

/// Starts a new Solo chat with [bestie] whose first message is [content].
Future<void> _quickSend(WidgetRef ref, String content, String bestie) async {
  if (content.isEmpty) throw _QuickSendRefused('Say a message.');
  if (utf8.encode(content).length > 8192) {
    throw _QuickSendRefused('That message is too long.');
  }
  // After its background grace period the session has closed the socket.
  // Reopen it for this send, then let the background pause start over.
  final session = ref.read(relaySessionProvider.notifier);
  final paused = switch (ref.read(appLifecycleProvider)) {
    AppLifecycleState.paused || AppLifecycleState.detached => true,
    _ => false,
  };
  if (paused) session.onAppResumed();
  try {
    await _sendToBestie(ref, content, bestie);
  } finally {
    if (paused && ref.read(appLifecycleProvider) != AppLifecycleState.resumed) {
      session.onAppPaused();
    }
  }
}

Future<void> _sendToBestie(WidgetRef ref, String content, String bestie) async {
  await _connected(ref);
  final sync = ref.read(meSyncProvider);
  if (MeChatCreation.pending(sync) != null) {
    throw _QuickSendRefused('Open Buzz to finish a chat that was interrupted.');
  }
  final channelId = const Uuid().v4();
  for (var attempt = 1; ; attempt++) {
    try {
      await _createMeChat(
        ref,
        sync,
        channelId,
        startingMeChatTitle(content),
        bestie,
        null,
        initialContent: content,
      );
      return;
    } on StateError {
      rethrow; // The agent or community changed; trying again can't help.
    } on MeRecoveryConflict {
      rethrow;
    } catch (_) {
      // A background launch often meets a dropped connection or, catching up
      // after a reconnect, the relay's write quota. Retrying is safe because
      // it resumes the saved creation with the same signed first message.
      if (attempt == 3) rethrow;
      await Future<void>.delayed(Duration(seconds: attempt));
      await _connected(ref);
    }
  }
}

/// Waits briefly for the relay session a cold background launch is opening.
Future<void> _connected(WidgetRef ref) async {
  if (ref.read(relaySessionProvider).status == SessionStatus.connected) return;
  final connected = Completer<void>();
  final subscription = ref.listenManual(
    relaySessionProvider.select((s) => s.status),
    (_, status) {
      if (status == SessionStatus.connected && !connected.isCompleted) {
        connected.complete();
      }
    },
    fireImmediately: true,
  );
  try {
    await connected.future.timeout(const Duration(seconds: 10));
  } on TimeoutException {
    throw _QuickSendRefused("Buzz couldn't reach your community.");
  } finally {
    subscription.close();
  }
}
