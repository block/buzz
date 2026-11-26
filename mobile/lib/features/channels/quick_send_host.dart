import 'dart:async';
import 'dart:convert';

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/me/default_agent.dart';
import '../../shared/relay/relay.dart';
import '../../shared/drafts/compose_drafts_provider.dart';
import 'bestie_conversation.dart';
import 'channel.dart';
import 'channel_management_provider.dart';
import 'send_message_provider.dart';

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
/// session and the Bestie's DM. It tells the platform once it can take sends.
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
            (
              'send',
              {
                'text': final String text,
                'bestie': final String to,
                'scope': final String scope,
              },
            ) =>
              await _quickSend(ref, text.trim(), to, scope),
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

/// The Bestie a quick send will go to.
Future<Map<String, String?>> _bestie(WidgetRef ref) async {
  // Captured first: a switch during the awaits must not pair this
  // community's scope with another's Bestie.
  final scope = _scope(ref);
  final bestie = await ref
      .read(defaultAgentProvider.future)
      .timeout(const Duration(seconds: 10));
  if (bestie == null) {
    throw const _QuickSendRefused('Choose a Bestie in Buzz first.');
  }
  _ensureScope(ref, scope);
  return {'pubkey': bestie, 'scope': scope};
}

/// The account and community a looked-up Bestie belongs to.
String _scope(WidgetRef ref) =>
    '${ref.read(relayConfigProvider).baseUrl} ${ref.read(myPubkeyProvider)}';

/// Refuses a send whose Bestie was looked up in another account or community.
void _ensureScope(WidgetRef ref, String scope) {
  if (_scope(ref) != scope) {
    throw const _QuickSendRefused('Buzz switched communities. Try again.');
  }
}

/// Posts [content] in the direct conversation with [bestie].
///
/// When the send fails or Buzz can't tell whether it landed, the message is
/// kept as that DM's composer draft so it isn't lost.
Future<void> _quickSend(
  WidgetRef ref,
  String content,
  String bestie,
  String scope,
) async {
  _ensureScope(ref, scope);
  // Bound to this community: each refuses to act after a switch.
  final actions = ref.read(channelActionsProvider);
  final send = ref.read(sendMessageProvider);
  final drafts = ref.read(composeDraftsProvider.notifier);
  if (content.isEmpty) throw const _QuickSendRefused('Say a message.');
  if (utf8.encode(content).length > 8192) {
    throw const _QuickSendRefused('That message is too long.');
  }
  var dm = cachedBestieConversation(ref, bestie);
  // After its background grace period the session has closed the socket.
  // Reopen it for this send, then let the background pause start over.
  final session = ref.read(relaySessionProvider.notifier);
  final paused = switch (ref.read(appLifecycleProvider)) {
    AppLifecycleState.paused || AppLifecycleState.detached => true,
    _ => false,
  };
  if (paused) session.onAppResumed();
  try {
    if (!await _connected(ref)) {
      if (dm == null) {
        throw const _QuickSendRefused(
          "Buzz couldn't reach your community, so your message wasn't saved.",
        );
      }
      _ensureScope(ref, scope);
      _keepDraft(drafts, dm, content);
      throw const _QuickSendRefused(
        "Buzz couldn't reach your community. Your message is a draft in "
        'your Bestie DM.',
      );
    }
    _ensureScope(ref, scope);
    try {
      dm ??= await actions.openDm(pubkeys: [bestie], waitForChannelList: false);
    } catch (failure) {
      debugPrint('Quick send lookup failed: $failure');
      throw const _QuickSendRefused(
        "Buzz couldn't open your Bestie DM, so your message wasn't saved.",
      );
    }
    for (var attempt = 1; ; attempt++) {
      try {
        await send(channelId: dm.id, content: content, channel: dm);
        return;
      } catch (failure) {
        // Catching up after a reconnect can use the relay's write quota. The
        // relay turned that message away, so sending again can't duplicate
        // it; any other failure might have landed and isn't retried.
        final limited = '$failure'.contains('rate-limited:');
        if (limited && attempt < 3) continue;
        debugPrint('Quick send failed: $failure');
        // A switch mid-send means this account's drafts are no longer here.
        _ensureScope(ref, scope);
        _keepDraft(drafts, dm, content);
        throw _QuickSendRefused(
          limited
              ? "Buzz couldn't send that yet. Your message is a draft in your "
                    'Bestie DM.'
              : "Buzz isn't sure that sent. Check your Bestie DM before "
                    'trying again.',
        );
      }
    }
  } finally {
    if (paused && ref.read(appLifecycleProvider) != AppLifecycleState.resumed) {
      session.onAppPaused();
    }
  }
}

/// Keeps [content] in [dm]'s composer draft, after any text already there.
void _keepDraft(ComposeDraftsNotifier drafts, Channel dm, String content) {
  final key = composeDraftKey(dm.id);
  final before = drafts.draftFor(key);
  drafts.save(
    key: key,
    channelId: dm.id,
    text: before == null ? content : '${before.text}\n\n$content',
    mentions: before?.mentions ?? const [],
    mentionsInvalid: before?.mentionsInvalid ?? false,
  );
}

/// Waits briefly for the relay session a cold background launch is opening.
Future<bool> _connected(WidgetRef ref) async {
  if (ref.read(relaySessionProvider).status == SessionStatus.connected) {
    return true;
  }
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
    return true;
  } on TimeoutException {
    return false;
  } finally {
    subscription.close();
  }
}
