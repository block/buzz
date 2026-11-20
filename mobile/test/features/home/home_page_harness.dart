import 'dart:async';

import 'package:buzz/features/channels/default_agent_launcher.dart';
import 'package:buzz/features/agents/agents_page.dart';
import 'package:buzz/features/channels/me/me_section.dart';
import 'package:buzz/features/activity/inbox_unread_provider.dart';
import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channels_provider.dart';
import 'package:buzz/shared/me/default_agent.dart';
import 'package:buzz/shared/relay/app_lifecycle_provider.dart';
import 'package:buzz/features/home/home_page.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/material.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';

/// Home with its providers stubbed, as every Home test mounts it.
Future<Widget> buildHome({
  bool longSolo = false,
  bool inboxUnread = false,
  bool disableAnimations = false,
  Gradient? topSectionGradient,
  ValueChanged<ValueNotifier<VoidCallback?>>? onNewChatAction,
  String? bestie,
  Stream<String?>? bestieUpdates,
  ValueChanged<String>? onOpenBestie,
}) async {
  SharedPreferences.setMockInitialValues({});
  final prefs = await SharedPreferences.getInstance();
  return ProviderScope(
    overrides: [
      savedPrefsProvider.overrideWithValue(prefs),
      inboxHasUnreadProvider.overrideWithValue(inboxUnread),
      defaultAgentProvider.overrideWith(
        (ref) => bestieUpdates ?? Stream.value(bestie),
      ),
      channelsProvider.overrideWith(_Channels.new),
      // Lifecycle changes here exercise Home, not the relay connection.
      appLifecycleProvider.overrideWith(_Lifecycle.new),
    ],
    child: MaterialApp(
      theme: AppTheme.light(topSectionGradient: topSectionGradient),
      builder: (context, child) => MediaQuery(
        data: MediaQuery.of(
          context,
        ).copyWith(disableAnimations: disableAnimations),
        child: child!,
      ),
      home: HomePage(
        onAgents: showAgents,
        openBestieConversation: (_, _, pubkey) async =>
            onOpenBestie?.call(pubkey),
        defaultAgentBuilder: (context) =>
            DefaultAgentLauncher(onChooseAgent: () => showAgents(context)),
        mePageBuilder: (_, action, workspace) {
          onNewChatAction?.call(action);
          return SliverToBoxAdapter(
            child: MeSection(
              label: 'Chats',
              children: [
                const Text('Personal workspace'),
                if (longSolo) ...[
                  const SizedBox(height: 1200),
                  const SizedBox(height: 44, child: Text('Last solo chat')),
                ],
              ],
            ),
          );
        },
        settingsPageBuilder: _buildSettingsPage,
      ),
    ),
  );
}

Widget _buildSettingsPage(BuildContext context) =>
    const Scaffold(body: Text('Your profile destination'));

class _Channels extends ChannelsNotifier {
  @override
  Future<List<Channel>> build() async => [
    Channel(
      id: 'channel',
      name: 'General',
      description: '',
      createdBy: 'owner',
      createdAt: DateTime(2026),
      memberCount: 1,
      visibility: 'open',
      channelType: 'stream',
      isMember: true,
    ),
  ];
}

class _Lifecycle extends AppLifecycleNotifier {
  @override
  AppLifecycleState build() => AppLifecycleState.resumed;
}
