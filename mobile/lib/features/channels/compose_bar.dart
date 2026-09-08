import 'dart:async';
import 'dart:collection';
import 'dart:io';
import 'dart:math' as math;
import 'dart:ui' show FlutterView;

import 'package:camera/camera.dart' as camera;
import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/physics.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter/services.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:image_picker/image_picker.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';

import 'package:nostr/nostr.dart' as nostr;

import '../../shared/mentions/agent_identity_provider.dart';
import '../../shared/huddle/huddle_session.dart';
import '../../shared/relay/relay.dart';
import '../../shared/theme/theme.dart';
import '../../shared/widgets/avatar_image.dart';
import '../../shared/widgets/anchored_popover_menu.dart';
import '../../shared/widgets/buzz_loading_indicator.dart';
import '../../shared/widgets/concentric_sheet_surface.dart';
import '../../shared/widgets/keyboard_dismiss_on_drag.dart';
import '../../shared/widgets/ios_glass_navigation_button.dart';
import '../../shared/widgets/mobile_tab_footer_backdrop.dart';
import '../../shared/widgets/modal_presentation.dart';
import '../../shared/profile/user_cache_provider.dart';
import '../../shared/profile/user_profile.dart';
import '../../shared/custom_emoji/custom_emoji.dart';
import '../../shared/custom_emoji/custom_emoji_provider.dart';
import '../../shared/deeplink/deep_link.dart';
import '../activity/compose_drafts_provider.dart';
import 'camera_capture_cleanup.dart';
import 'channel.dart';
import 'channel_management_provider.dart';
import 'channels_provider.dart';
import 'emoji_picker.dart';
import 'mentions/mention_candidates.dart';
import 'mentions/mention_candidates_provider.dart';
import 'mentions/mention_ranking.dart';
import 'photo_library.dart';
import 'voice_note_attachment.dart';
import 'voice_note_composer_recorder.dart';
import 'voice_note_recording.dart';

part 'compose_bar/helpers.dart';
part 'compose_bar/agent_mention_labels.dart';
part 'compose_bar/markdown_editing_controller.dart';
part 'compose_bar/draft_lifecycle.dart';
part 'compose_bar/suggestions.dart';
part 'compose_bar/formatting_toolbar.dart';
part 'compose_bar/attachments.dart';
part 'compose_bar/upload_progress_pill.dart';
part 'compose_bar/photo_gallery_picker.dart';
part 'compose_bar/ios_photo_picker.dart';
part 'compose_bar/ios_attachment_popover.dart';
part 'compose_bar/camera_preview.dart';
part 'compose_bar/send_button.dart';
part 'compose_bar/voice_note.dart';
part 'compose_bar/layout.dart';
part 'compose_bar/dock.dart';
part 'compose_bar/compose_bar_widget.dart';

/// Callback used by channels and threads to submit composer content.
typedef ComposeBarOnSend =
    Future<void> Function(
      String content,
      List<String> mentionPubkeys, {
      List<List<String>> mediaTags,
    });
