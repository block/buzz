import 'package:flutter/material.dart';

import 'grid.dart';
import 'text_theme.dart';

const _fontFamily = 'Inter';
const _messageLetterSpacing = -0.45;

/// Avatar size for full channel and thread messages.
const messageAvatarSize = 36.0;

/// Avatar size for conversation-oriented Activity rows.
const activityAvatarSize = 42.0;

/// Avatar size for compact message-result rows.
const compactMessageAvatarSize = 42.0;

/// Horizontal space between a message avatar and its content.
const messageAvatarContentGap = Grid.twelve;

/// Horizontal inset shared by bubble text and its author line.
const messageBubbleHorizontalInset = 14.0;
const messageBubbleVerticalInset = 10.0;

/// Maximum text-bubble width relative to the screen, including padding.
const messageTextMaxWidthFraction = 2 / 3;

/// Shared by drafts and sent messages so their line spacing matches.
const messageBodyTextStyle = TextStyle(
  fontFamily: _fontFamily,
  fontSize: 17,
  fontWeight: FontWeight.w400,
  height: 23 / 17,
  letterSpacing: _messageLetterSpacing,
);

/// Message author names: 15sp semibold on a 17sp line height.
const messageUsernameTextStyle = TextStyle(
  fontFamily: _fontFamily,
  fontSize: 15,
  fontWeight: FontWeight.w600,
  height: 17 / 15,
  letterSpacing: 0,
);

/// Secondary author metadata: 15sp regular on a tight 17sp line height.
const messageMetadataTextStyle = TextStyle(
  fontFamily: _fontFamily,
  fontSize: 15,
  fontWeight: FontWeight.w400,
  height: 17 / 15,
  letterSpacing: 0,
);

/// Message timestamps: 13.1sp regular, one step below 15sp author names.
const messageTimestampTextStyle = TextStyle(
  fontFamily: _fontFamily,
  fontSize: 13.1,
  fontWeight: FontWeight.w400,
  height: 17 / 13.1,
  letterSpacing: 0,
);

/// Reaction counts: 13.1sp medium on a 17sp line height.
const reactionCountTextStyle = TextStyle(
  fontFamily: _fontFamily,
  fontSize: 13.1,
  fontWeight: FontWeight.w500,
  height: 17 / 13.1,
  letterSpacing: 0,
);

/// Channel and thread titles share the medium title role.
const channelTitleTextStyle = titleMediumTextStyle;

const messageBylineTextStyle = bodyExtraSmallTextStyle;
const messageCaptionTextStyle = messageTimestampTextStyle;

/// Primary labels in compact content lists.
const contentListTitleTextStyle = messageUsernameTextStyle;

/// Titles shared by Home channel rows and Solo chat rows.
const navigationChatTitleTextStyle = TextStyle(
  fontFamily: _fontFamily,
  letterSpacing: _messageLetterSpacing,
  fontSize: 17,
  height: 20 / 15,
  fontWeight: FontWeight.w500,
);

/// Secondary copy in compact content lists.
const contentListBodyTextStyle = TextStyle(
  fontFamily: _fontFamily,
  fontSize: 13.1,
  fontWeight: FontWeight.w400,
  height: 17 / 13.1,
  letterSpacing: 0,
);

/// Filter chip labels use a tighter 15sp Inter treatment.
const filterChipTextStyle = TextStyle(
  fontFamily: _fontFamily,
  fontSize: 15,
  fontWeight: FontWeight.w400,
  height: 1,
  letterSpacing: 0,
);

/// Search fields use the primary 15sp body treatment.
const searchInputTextStyle = bodyLargeTextStyle;

/// Activity sender names share the primary author style.
const activityUsernameTextStyle = messageUsernameTextStyle;

/// Activity timestamps share the compact message timestamp treatment.
const activityTimestampTextStyle = messageTimestampTextStyle;

/// Activity context labels: 13.1sp medium on a 17sp line height.
const activityContextTextStyle = TextStyle(
  fontFamily: _fontFamily,
  fontSize: 13.1,
  fontWeight: FontWeight.w500,
  height: 17 / 13.1,
  letterSpacing: 0,
);

/// Activity message previews use the primary message copy style.
const activityPreviewTextStyle = bodyLargeTextStyle;

/// Inventory labels shared by Solo and Team navigation sections.
const navigationSectionTextStyle = TextStyle(
  fontFamily: _fontFamily,
  fontSize: 15,
  height: 20 / 15,
  fontWeight: FontWeight.w500,
);

TextStyle navigationBucketHeadingStyle(BuildContext context) =>
    navigationChatTitleTextStyle.copyWith(
      fontWeight: FontWeight.w600,
      color: Theme.of(context).colorScheme.onSurface,
    );
