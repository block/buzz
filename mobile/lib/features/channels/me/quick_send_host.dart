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
            message: failure.toString().replaceFirst('Bad state: ', ''),
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
  await _connected(ref);
  final sync = ref.read(meSyncProvider);
  if (MeChatCreation.pending(sync) != null) {
    throw StateError('Open Buzz to finish a chat that was interrupted.');
  }
  final bestie = await ref
      .read(defaultAgentProvider.future)
      .timeout(const Duration(seconds: 10));
  if (bestie == null) throw StateError('Choose a Bestie in Buzz first.');
  await _createMeChat(
    ref,
    sync,
    const Uuid().v4(),
    startingMeChatTitle(content),
    bestie,
    null,
    initialContent: content,
  );
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
