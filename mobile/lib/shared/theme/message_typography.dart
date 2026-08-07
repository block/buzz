import 'package:flutter/material.dart';

import 'grid.dart';

const _fontFamily = 'Inter';

/// Avatar size for full channel and thread messages.
const messageAvatarSize = 42.0;

/// Avatar size for conversation-oriented Activity rows.
const activityAvatarSize = messageAvatarSize;

/// Avatar size for compact message-result rows.
const compactMessageAvatarSize = messageAvatarSize;

/// Horizontal space between a message avatar and its content.
const messageAvatarContentGap = Grid.twelve;

/// Primary message copy: 15sp regular on a 20sp line height.
const messageBodyTextStyle = TextStyle(
  fontFamily: _fontFamily,
  fontSize: 15,
  fontWeight: FontWeight.w400,
  height: 20 / 15,
  letterSpacing: 0,
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

/// Message timestamps share the secondary author metadata style.
const messageTimestampTextStyle = messageMetadataTextStyle;

/// Compact reply previews: 13.1sp regular on a 17sp line height.
const replyPreviewTextStyle = TextStyle(
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

/// Channel and thread titles: 20sp bold on a 24sp line height.
const channelTitleTextStyle = TextStyle(
  fontFamily: _fontFamily,
  fontSize: 20,
  fontWeight: FontWeight.w700,
  height: 24 / 20,
  letterSpacing: 0,
);

/// Primary labels in compact content lists.
const contentListTitleTextStyle = messageUsernameTextStyle;

/// Secondary copy in compact content lists.
const contentListBodyTextStyle = TextStyle(
  fontFamily: _fontFamily,
  fontSize: 13.1,
  fontWeight: FontWeight.w400,
  height: 17 / 13.1,
  letterSpacing: 0,
);

/// Timestamps in compact content lists.
const contentListTimestampTextStyle = messageMetadataTextStyle;

/// Filter chip labels use a tighter 15sp Inter treatment.
const filterChipTextStyle = TextStyle(
  fontFamily: _fontFamily,
  fontSize: 15,
  fontWeight: FontWeight.w400,
  height: 1,
  letterSpacing: 0,
);

/// Search fields use the primary 15sp body treatment.
const searchInputTextStyle = messageBodyTextStyle;

/// System message actor names: 15sp semibold on a 17sp line height.
const systemMessageHeadingTextStyle = TextStyle(
  fontFamily: _fontFamily,
  fontSize: 15,
  fontWeight: FontWeight.w600,
  height: 17 / 15,
  letterSpacing: 0,
);

/// System message copy: 15sp regular on a 20sp line height.
const systemMessageBodyTextStyle = TextStyle(
  fontFamily: _fontFamily,
  fontSize: 15,
  fontWeight: FontWeight.w400,
  height: 20 / 15,
  letterSpacing: 0,
);

/// Activity sender names share the primary author style.
const activityUsernameTextStyle = messageUsernameTextStyle;

/// Activity timestamps share the secondary author metadata style.
const activityTimestampTextStyle = messageMetadataTextStyle;

/// Activity context labels: 13.1sp medium on a 17sp line height.
const activityContextTextStyle = TextStyle(
  fontFamily: _fontFamily,
  fontSize: 13.1,
  fontWeight: FontWeight.w500,
  height: 17 / 13.1,
  letterSpacing: 0,
);

/// Activity message previews use the primary message copy style.
const activityPreviewTextStyle = messageBodyTextStyle;
