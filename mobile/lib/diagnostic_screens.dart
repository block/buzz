import 'package:flutter/widgets.dart';

import 'features/activity/activity_page.dart';
import 'features/agents/agents_page.dart';
import 'features/agents/create_agent_page.dart';
import 'features/channels/channel_actions_sheet.dart';
import 'features/channels/channel_detail_page.dart';
import 'features/channels/channels_page.dart';
import 'features/channels/media_viewer_page.dart';
import 'features/channels/thread_detail_page.dart';
import 'features/forum/forum_thread_page.dart';
import 'features/home/home_page.dart';
import 'features/invites/invite_create_page.dart';
import 'features/onboarding/hosted_connect_page.dart';
import 'features/pairing/pairing_page.dart';
import 'features/profile/profile_edit_page.dart';
import 'features/search/search_page.dart';
import 'features/settings/settings_page.dart';
import 'features/settings/theme_picker_page.dart';
import 'onboarding_app_entry.dart';
import 'organization_app_entry.dart';
import 'shared/diagnostics/diagnostic_reporter.dart';

/// The app's pages that have a name in a report's trail of screens, for
/// `diagnosticScreenClassifierProvider`. Add a page here to name it. A route is
/// named by the first page below it that this names, so a page that wraps
/// another listed page names both.
DiagnosticScreen? diagnosticScreenOf(Widget page) => switch (page) {
  HomePage() => DiagnosticScreen.channelList,
  ChannelDetailPage() => DiagnosticScreen.channel,
  ThreadDetailPage() => DiagnosticScreen.thread,
  ForumThreadPage() => DiagnosticScreen.forumPost,
  ChannelDetailsPage() => DiagnosticScreen.channelDetails,
  SearchPage() => DiagnosticScreen.search,
  ActivityPage() => DiagnosticScreen.activity,
  SettingsPage() => DiagnosticScreen.settings,
  ThemePickerPage() => DiagnosticScreen.appearance,
  ProfileEditPage() => DiagnosticScreen.profile,
  AgentsPage() => DiagnosticScreen.agents,
  CreateAgentPage() => DiagnosticScreen.createAgent,
  CommunityInvitePage() => DiagnosticScreen.invites,
  CommunitySwitcherPage() => DiagnosticScreen.communitySwitcher,
  OnboardingAppEntry(addingCommunity: true) ||
  PairingPage(
    addingCommunity: true,
    identityRecoveryOnly: false,
  ) => DiagnosticScreen.addCommunity,
  MediaImageViewerPage() ||
  MediaVideoViewerPage() => DiagnosticScreen.mediaViewer,
  OnboardingAppEntry() ||
  OrganizationAppEntry() ||
  PairingPage() ||
  HostedSetupSteps() => DiagnosticScreen.onboarding,
  _ => null,
};
