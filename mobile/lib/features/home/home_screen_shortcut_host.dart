import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/me/default_agent.dart';
import '../../shared/profile/user_cache_provider.dart';
import '../../shared/shortcuts/home_screen_shortcuts.dart';

/// Publishes Home's app icon quick actions and runs a tapped one.
///
/// It is a separate leaf so the Bestie, lifecycle and pending-tap state it
/// watches rebuild only this widget, never Home.
class HomeScreenShortcutHost extends HookConsumerWidget {
  const HomeScreenShortcutHost({
    required this.newChat,
    required this.newDm,
    required this.onAgents,
    required this.openBestieConversation,
    super.key,
  });

  final ValueListenable<VoidCallback?> newChat;
  final ValueListenable<VoidCallback?> newDm;
  final ValueChanged<BuildContext> onAgents;
  final Future<void> Function(BuildContext, WidgetRef, String pubkey)
  openBestieConversation;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final startChat = useValueListenable(newChat);
    final startDm = useValueListenable(newDm);
    final bestie = ref.watch(defaultAgentProvider);
    final bestiePubkey = bestie.isLoading || bestie.hasError
        ? null
        : bestie.value;
    final bestieName = ref.watch(
      userCacheProvider.select((profiles) => profiles[bestiePubkey]?.label),
    );
    final published = useRef<List<String>?>(null);
    useEffect(() {
      unawaited(listenForHomeScreenShortcuts());
      // Signing out removes Home, and its shortcuts with it.
      return () => unawaited(publishHomeScreenShortcuts(const []));
    }, const []);
    useEffect(() {
      // A failed read can hold another community's Bestie, so keep the
      // last published menu until this community's Bestie is known.
      if (bestie.isLoading || bestie.hasError) return null;
      // New chat and New DM stay listed while their actions are briefly
      // unavailable (loading, or a New chat already open); a tap waits.
      final items = [
        (shortcut: HomeScreenShortcut.newChat, title: 'New chat'),
        (shortcut: HomeScreenShortcut.newDm, title: 'New DM'),
        if (bestiePubkey != null)
          (
            shortcut: HomeScreenShortcut.messageBestie,
            title: 'Message ${bestieName ?? 'your Bestie'}',
          ),
      ];
      final key = [for (final item in items) item.title];
      // Each reconnect reloads the Bestie; skip republishing the same menu.
      if (listEquals(key, published.value)) return null;
      published.value = key;
      unawaited(publishHomeScreenShortcuts(items));
      return null;
    }, [bestie.isLoading, bestie.hasError, bestiePubkey, bestieName]);
    final pendingShortcut = useValueListenable(pendingHomeScreenShortcut);
    // The tap Home already returned to, so a waiting tap never pops screens
    // the user opens while it waits.
    final poppedFor = useRef<HomeScreenShortcut?>(null);
    if (pendingShortcut == null) poppedFor.value = null;
    // Covering Home pauses this leaf's providers, so a screen opened while a
    // tap waits is noticed through the route instead.
    final homeCurrent = ModalRoute.of(context)?.isCurrent ?? true;
    // A tap arrives while iOS is still bringing the app forward, before
    // native sheets such as New DM can present.
    final resumed =
        (useAppLifecycleState() ?? WidgetsBinding.instance.lifecycleState) ==
        AppLifecycleState.resumed;
    useEffect(
      () {
        if (pendingShortcut == null || !resumed) return null;
        final VoidCallback? run = switch (pendingShortcut) {
          HomeScreenShortcut.newChat => startChat,
          HomeScreenShortcut.newDm => startDm,
          // Wait for this community's Bestie before choosing a destination.
          HomeScreenShortcut.messageBestie
              when bestie.isLoading || bestie.hasError =>
            null,
          HomeScreenShortcut.messageBestie => switch (bestiePubkey) {
            // A shortcut published for another community's Bestie.
            null => () => onAgents(context),
            final pubkey => () => unawaited(
              openBestieConversation(context, ref, pubkey),
            ),
          },
        };
        // Navigation and notifier writes must wait until this build finishes.
        WidgetsBinding.instance.addPostFrameCallback((_) {
          if (!context.mounted ||
              WidgetsBinding.instance.lifecycleState !=
                  AppLifecycleState.resumed ||
              pendingHomeScreenShortcut.value != pendingShortcut) {
            return;
          }
          if (poppedFor.value != pendingShortcut) {
            poppedFor.value = pendingShortcut;
            // Each shortcut starts from Home, so it never opens beneath a
            // sheet or a conversation left open earlier. Leaving an open
            // New chat draft also makes New chat available again. A screen
            // that refuses to close, such as a first message still sending,
            // stays.
            Navigator.of(context).popUntil(
              (route) =>
                  route.isFirst ||
                  (route is ModalRoute &&
                      route.popDisposition == RoutePopDisposition.doNotPop),
            );
          }
          // Drop the tap when a screen kept Home covered or opened over it
          // while the tap waited.
          if (ModalRoute.of(context)?.isCurrent == false) {
            poppedFor.value = null;
            pendingHomeScreenShortcut.value = null;
            return;
          }
          if (run == null) return;
          poppedFor.value = null;
          pendingHomeScreenShortcut.value = null;
          run();
        });
        return null;
      },
      [
        pendingShortcut,
        resumed,
        homeCurrent,
        startChat,
        startDm,
        bestie.isLoading,
        bestie.hasError,
        bestiePubkey,
      ],
    );
    return const SizedBox.shrink();
  }
}
