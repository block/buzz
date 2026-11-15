part of 'me_page.dart';

const _quickSendChannel = MethodChannel('buzz/quick_send');

/// Answers quick sends from Siri and Shortcuts while Home is mounted.
///
/// iOS can run a quick send with Buzz in the background; the app still
/// builds Home then, so this leaf is where the send gets the community's
/// session and Bestie. It tells the platform once it can take sends.
class QuickSendHost extends HookConsumerWidget {
  const QuickSendHost({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    useEffect(() {
      _quickSendChannel.setMethodCallHandler((call) async {
        if (call.method != 'send') throw MissingPluginException();
        final text = call.arguments;
        if (text is! String || text.trim().isEmpty) {
          throw PlatformException(code: 'empty', message: 'Say a message.');
        }
        try {
          await quickSendToBestie(ref, text.trim());
        } on PlatformException {
          rethrow;
        } catch (failure) {
          throw PlatformException(
            code: 'failed',
            message: failure.toString().replaceFirst(
              RegExp(r'^(Bad state|Exception): '),
              '',
            ),
          );
        }
      });
      unawaited(_quickSendChannel.invokeMethod<void>('ready'));
      return () => _quickSendChannel.setMethodCallHandler(null);
    }, const []);
    return const SizedBox.shrink();
  }
}

/// Starts a new Solo chat with the Bestie whose first message is [content].
@visibleForTesting
Future<void> quickSendToBestie(WidgetRef ref, String content) async {
  if (utf8.encode(content).length > 8192) {
    throw StateError('That message is too long.');
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
    await _sendToBestie(ref, content);
  } finally {
    if (paused && ref.read(appLifecycleProvider) != AppLifecycleState.resumed) {
      session.onAppPaused();
    }
  }
}

Future<void> _sendToBestie(WidgetRef ref, String content) async {
  await _connected(ref);
  final sync = ref.read(meSyncProvider);
  if (MeChatCreation.pending(sync) != null) {
    throw StateError('Open Buzz to finish a chat that was interrupted.');
  }
  final bestie = await ref
      .read(defaultAgentProvider.future)
      .timeout(const Duration(seconds: 10));
  if (bestie == null) throw StateError('Choose a Bestie in Buzz first.');
  final channelId = const Uuid().v4();
  for (var attempt = 1; ; attempt++) {
    try {
      // A retry resumes the saved creation, with the same first message.
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
    } catch (failure) {
      // Catching up after a reconnect can briefly use the relay's write quota.
      if (attempt == 3 || !'$failure'.contains('rate-limited:')) rethrow;
      await Future<void>.delayed(Duration(seconds: attempt));
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
    throw StateError("Buzz couldn't reach your community.");
  } finally {
    subscription.close();
  }
}
