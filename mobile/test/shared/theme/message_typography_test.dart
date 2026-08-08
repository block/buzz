import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  void expectStyle(
    TextStyle style, {
    required double fontSize,
    required FontWeight fontWeight,
    required double lineHeight,
    required double letterSpacing,
  }) {
    expect(style.fontFamily, 'Inter');
    expect(style.fontSize, fontSize);
    expect(style.fontWeight, fontWeight);
    expect(style.height, closeTo(lineHeight / fontSize, 0.0001));
    expect(style.letterSpacing, letterSpacing);
  }

  test('message typography matches the shared mobile scale', () {
    expectStyle(
      messageBodyTextStyle,
      fontSize: 15,
      fontWeight: FontWeight.w400,
      lineHeight: 20,
      letterSpacing: 0,
    );
    expectStyle(
      messageUsernameTextStyle,
      fontSize: 15,
      fontWeight: FontWeight.w600,
      lineHeight: 17,
      letterSpacing: 0,
    );
    expectStyle(
      messageTimestampTextStyle,
      fontSize: 15,
      fontWeight: FontWeight.w400,
      lineHeight: 17,
      letterSpacing: 0,
    );
    expect(messageTimestampTextStyle, messageMetadataTextStyle);
    expectStyle(
      replyPreviewTextStyle,
      fontSize: 13.1,
      fontWeight: FontWeight.w400,
      lineHeight: 17,
      letterSpacing: 0,
    );
    expectStyle(
      reactionCountTextStyle,
      fontSize: 13.1,
      fontWeight: FontWeight.w500,
      lineHeight: 17,
      letterSpacing: 0,
    );
    expectStyle(
      channelTitleTextStyle,
      fontSize: 20,
      fontWeight: FontWeight.w700,
      lineHeight: 24,
      letterSpacing: 0,
    );
    expectStyle(
      systemMessageHeadingTextStyle,
      fontSize: 15,
      fontWeight: FontWeight.w600,
      lineHeight: 17,
      letterSpacing: 0,
    );
    expectStyle(
      systemMessageBodyTextStyle,
      fontSize: 15,
      fontWeight: FontWeight.w400,
      lineHeight: 20,
      letterSpacing: 0,
    );
  });

  test('activity typography matches the conversation row scale', () {
    expectStyle(
      activityUsernameTextStyle,
      fontSize: 15,
      fontWeight: FontWeight.w600,
      lineHeight: 17,
      letterSpacing: 0,
    );
    expectStyle(
      activityTimestampTextStyle,
      fontSize: 15,
      fontWeight: FontWeight.w400,
      lineHeight: 17,
      letterSpacing: 0,
    );
    expectStyle(
      activityContextTextStyle,
      fontSize: 13.1,
      fontWeight: FontWeight.w500,
      lineHeight: 17,
      letterSpacing: 0,
    );
    expectStyle(
      activityPreviewTextStyle,
      fontSize: 15,
      fontWeight: FontWeight.w400,
      lineHeight: 20,
      letterSpacing: 0,
    );
  });

  test('content list typography matches the compact list scale', () {
    expectStyle(
      contentListTitleTextStyle,
      fontSize: 15,
      fontWeight: FontWeight.w600,
      lineHeight: 17,
      letterSpacing: 0,
    );
    expectStyle(
      contentListBodyTextStyle,
      fontSize: 13.1,
      fontWeight: FontWeight.w400,
      lineHeight: 17,
      letterSpacing: 0,
    );
    expectStyle(
      contentListTimestampTextStyle,
      fontSize: 15,
      fontWeight: FontWeight.w400,
      lineHeight: 17,
      letterSpacing: 0,
    );
    expectStyle(
      filterChipTextStyle,
      fontSize: 15,
      fontWeight: FontWeight.w400,
      lineHeight: 15,
      letterSpacing: 0,
    );
  });

  test('message and activity avatars use their surface sizes', () {
    expect(messageAvatarSize, 42);
    expect(activityAvatarSize, 42);
    expect(compactMessageAvatarSize, 42);
    expect(messageAvatarContentGap, Grid.twelve);
  });
}
