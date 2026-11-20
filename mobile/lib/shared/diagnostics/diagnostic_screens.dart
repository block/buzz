import 'package:flutter/widgets.dart';

import '../../features/activity/activity_page.dart';
import '../../features/agents/agents_page.dart';
import '../../features/channels/channel_actions_sheet.dart';
import '../../features/channels/channel_detail_page.dart';
import '../../features/channels/channels_page.dart';
import '../../features/channels/media_viewer_page.dart';
import '../../features/channels/thread_detail_page.dart';
import '../../features/forum/forum_thread_page.dart';
import '../../features/home/home_page.dart';
import '../../features/invites/invite_create_page.dart';
import '../../features/onboarding/hosted_connect_page.dart';
import '../../features/pairing/add_community_page.dart';
import '../../features/pairing/pairing_page.dart';
import '../../features/profile/profile_edit_page.dart';
import '../../features/search/search_page.dart';
import '../../features/settings/settings_page.dart';
import '../../features/settings/theme_picker_page.dart';
import '../../onboarding_app_entry.dart';
import '../../organization_app_entry.dart';
import 'diagnostic_reporter.dart';

/// Names the screen a route shows, for the trail of screens in a report.
///
/// A route has no name of its own here: the app pushes page widgets without
/// route settings, and where settings exist they can carry IDs. So a route is
/// named by the first page widget below it that is on this list, and a route
/// showing none is [DiagnosticScreen.other]. Nothing is read from a route's
/// settings, its arguments or what the page shows.
abstract final class DiagnosticScreens {
  /// The app's pages that have a name. Add a page here to name it; an unlisted
  /// page is reported as [DiagnosticScreen.other], never by its own type name,
  /// which release builds can obfuscate.
  static const pages = <Type, DiagnosticScreen>{
    HomePage: DiagnosticScreen.channelList,
    ChannelDetailPage: DiagnosticScreen.channel,
    ThreadDetailPage: DiagnosticScreen.thread,
    ForumThreadPage: DiagnosticScreen.forumPost,
    ChannelDetailsPage: DiagnosticScreen.channelDetails,
    SearchPage: DiagnosticScreen.search,
    ActivityPage: DiagnosticScreen.activity,
    SettingsPage: DiagnosticScreen.settings,
    ThemePickerPage: DiagnosticScreen.appearance,
    ProfileEditPage: DiagnosticScreen.profile,
    AgentsPage: DiagnosticScreen.agents,
    CommunityInvitePage: DiagnosticScreen.invites,
    CommunitySwitcherPage: DiagnosticScreen.communitySwitcher,
    AddCommunityPage: DiagnosticScreen.addCommunity,
    MediaImageViewerPage: DiagnosticScreen.mediaViewer,
    MediaVideoViewerPage: DiagnosticScreen.mediaViewer,
    OnboardingAppEntry: DiagnosticScreen.onboarding,
    OrganizationAppEntry: DiagnosticScreen.onboarding,
    PairingPage: DiagnosticScreen.onboarding,
    HostedConnectPage: DiagnosticScreen.onboarding,
  };

  /// How many elements a search below a route may look at. A page sits near
  /// the top of its route; this keeps an unlisted page from costing a walk of
  /// everything it shows.
  static const _searchBudget = 3000;

  /// The screen [route] shows, once it has been built.
  static DiagnosticScreen of(ModalRoute<dynamic> route) {
    if (route.subtreeContext case final BuildContext context) {
      var budget = _searchBudget;
      DiagnosticScreen? found;
      void visit(Element element) {
        if (found != null || budget-- <= 0) return;
        found = pages[element.widget.runtimeType];
        if (found == null) element.visitChildElements(visit);
      }

      (context as Element).visitChildElements(visit);
      if (found != null) return found!;
    }
    // The first route of a nested navigator is its container, such as the
    // empty list side of a pane, and shows whatever hosts that navigator.
    if (route.isFirst) {
      final navigator = route.navigator;
      final host = navigator == null ? null : ModalRoute.of(navigator.context);
      if (host != null) return of(host);
    }
    return DiagnosticScreen.other;
  }
}
