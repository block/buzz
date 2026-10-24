import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import 'features/channels/deep_link_dispatcher.dart';
import 'features/profile/profile_setup_page.dart';
import 'features/onboarding/hosted_connect_page.dart';
import 'features/onboarding/self_hosted_entry_sheet.dart';
import 'shared/auth/auth.dart';
import 'shared/theme/grid.dart';
import 'shared/deeplink/deep_link.dart';
import 'shared/hosted/hosted_community_connection.dart';
import 'shared/hosted/hosted_flow.dart';
import 'shared/hosted/hosted_environment.dart';
import 'features/onboarding/hosted_reconnect_banner.dart';

import 'shared/hosted/hosted_flow_provider.dart';

/// Composition seam shared by signed-in and signed-out application states.
class HostedAppGate extends HookConsumerWidget {
  const HostedAppGate({
    super.key,
    required this.child,
    required this.authenticated,
  });
  final Widget child;
  final bool authenticated;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final flow = HostedEnvironment.enabled
        ? ref.watch(hostedFlowProvider)
        : null;
    useListenable(flow);
    useEffect(() {
      if (flow == null) return null;
      void onFlowChanged() {
        if (flow.stage == HostedFlowStage.complete) {
          ref.read(hostedInlineEntryProvider.notifier).setActive(false);
        }
      }

      flow.addListener(onFlowChanged);
      unawaited(flow.initialize());
      return () => flow.removeListener(onFlowChanged);
    }, [flow]);
    final inlineEntry = ref.watch(hostedInlineEntryProvider);
    final visible =
        flow != null &&
        flow.stage != HostedFlowStage.idle &&
        flow.stage != HostedFlowStage.complete;
    // Native browser authentication presents its own alert over the current
    // screen. Keep that screen in place while checking the session/opening it.
    final openingSignIn =
        flow != null &&
        (flow.stage == HostedFlowStage.signIn ||
            (flow.accountOnly &&
                flow.stage == HostedFlowStage.working &&
                flow.account.account == null));
    final inlineAccountStatus =
        flow != null &&
        (!authenticated || inlineEntry) &&
        flow.accountOnly &&
        (flow.stage == HostedFlowStage.working ||
            (flow.stage == HostedFlowStage.blocked &&
                !flow.canChangeCommunities));
    return DeepLinkDispatcher(
      dispatchMessageLinks: authenticated && !visible,
      onboardingHandler: (routeContext, link) async {
        final relay = link.relayUrl;
        if (relay != null) {
          final host = Uri.parse(relay).host.toLowerCase();
          final requiresAuth = HostedEnvironment.requiresAuth(host);
          if (link is InviteDeepLink && !requiresAuth) return false;
          final communities = await ref
              .read(communityStorageProvider)
              .loadAll();
          final existing = communities
              .where(
                (item) => relayIdentity(item.relayUrl) == relayIdentity(relay),
              )
              .firstOrNull;
          if (existing != null &&
              !(existing.starterSetupIncomplete && requiresAuth)) {
            await ref
                .read(communityListProvider.notifier)
                .switchCommunity(existing.id);
            if (context.mounted) {
              ref.read(communityEntryCompletionProvider.notifier).complete();
            }
            return true;
          }
          if (!requiresAuth && link is AddCommunityDeepLink) {
            if (routeContext.mounted) {
              final joined = await showSelfHostedEntry(
                routeContext,
                relay: link.relayUrl,
                name: link.name,
              );
              if (joined && context.mounted) {
                ref.read(communityEntryCompletionProvider.notifier).complete();
              }
            }
            return true;
          }
          if (requiresAuth &&
              !HostedEnvironment.supportsRelay(Uri.parse(relay))) {
            if (routeContext.mounted) {
              await showDialog<void>(
                context: routeContext,
                builder: (context) => AlertDialog(
                  title: const Text('Community setup unavailable'),
                  content: const Text(
                    'This community needs a different or unsupported Builderlab environment. Use an app build configured for this community.',
                  ),
                  actions: [
                    TextButton(
                      onPressed: () => Navigator.pop(context),
                      child: const Text('Close'),
                    ),
                  ],
                ),
              );
            }
            return true;
          }
        }
        if (flow == null) return false;
        await flow.start(link);
        return true;
      },
      child: Stack(
        fit: StackFit.expand,
        children: [
          child,
          if (flow != null) ...[
            if (authenticated && !visible) HostedReconnectBanner(flow: flow),
            if (flow.stage == HostedFlowStage.signIn &&
                authenticated &&
                !inlineEntry)
              Positioned(
                left: Grid.sm,
                right: Grid.sm,
                bottom: 0,
                child: SafeArea(
                  child: Material(
                    color: Colors.transparent,
                    child: Wrap(
                      alignment: WrapAlignment.center,
                      children: [
                        TextButton(
                          onPressed: flow.retry,
                          child: const Text('Start login again'),
                        ),
                        TextButton(
                          onPressed: flow.leave,
                          child: Text(
                            flow.reconnecting
                                ? 'Cancel sign-in'
                                : 'Leave setup',
                          ),
                        ),
                      ],
                    ),
                  ),
                ),
              ),
            if (flow.stage == HostedFlowStage.profile)
              ProfileSetupPage(flow: flow),
            if (visible &&
                flow.stage != HostedFlowStage.profile &&
                !openingSignIn &&
                !inlineAccountStatus &&
                (!inlineEntry ||
                    flow.stage == HostedFlowStage.chooseIdentity ||
                    flow.stage == HostedFlowStage.chooseCommunities ||
                    flow.stage == HostedFlowStage.communityConflict ||
                    flow.canChangeCommunities))
              HostedConnectPage(
                flow: flow,
                onClose: () => unawaited(flow.leave()),
              ),
          ],
        ],
      ),
    );
  }
}
