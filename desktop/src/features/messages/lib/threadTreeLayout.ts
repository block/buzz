const THREAD_REPLY_MAX_VISIBLE_DEPTH = 6;

const THREAD_REPLY_AVATAR_SIZE_PX = 40;
const THREAD_REPLY_ROW_CONTENT_INSET_PX = 12;
const THREAD_REPLY_ROW_CONTENT_GAP_PX = 10;
const THREAD_REPLY_ROW_PADDING_TOP_PX = 8;
const THREAD_REPLY_AVATAR_RADIUS_PX = THREAD_REPLY_AVATAR_SIZE_PX / 2;
const THREAD_REPLY_AVATAR_LINE_GAP_PX = 4;

export const THREAD_REPLY_BODY_OFFSET_PX =
  THREAD_REPLY_ROW_CONTENT_INSET_PX +
  THREAD_REPLY_AVATAR_SIZE_PX +
  THREAD_REPLY_ROW_CONTENT_GAP_PX;
export const THREAD_REPLY_ROOT_INDENT_PX =
  THREAD_REPLY_BODY_OFFSET_PX - THREAD_REPLY_ROW_CONTENT_INSET_PX;
export const THREAD_REPLY_NESTED_INDENT_PX = THREAD_REPLY_ROOT_INDENT_PX;
export const THREAD_REPLY_LINE_WIDTH_PX = 1.5;

const THREAD_REPLY_AVATAR_CENTER_OFFSET_PX =
  THREAD_REPLY_ROW_CONTENT_INSET_PX + THREAD_REPLY_AVATAR_SIZE_PX / 2;
const THREAD_REPLY_AVATAR_CENTER_Y_PX =
  THREAD_REPLY_ROW_PADDING_TOP_PX + THREAD_REPLY_AVATAR_SIZE_PX / 2;

function clampVisibleDepth(depth: number) {
  return Math.min(Math.max(depth, 0), THREAD_REPLY_MAX_VISIBLE_DEPTH);
}

export function getThreadReplyIndentPx(depth: number) {
  const visibleDepth = clampVisibleDepth(depth);
  return visibleDepth > 0
    ? THREAD_REPLY_ROOT_INDENT_PX +
        (visibleDepth - 1) * THREAD_REPLY_NESTED_INDENT_PX
    : 0;
}

export function getThreadReplyAvatarCenterPx(depth: number) {
  return getThreadReplyIndentPx(depth) + THREAD_REPLY_AVATAR_CENTER_OFFSET_PX;
}

export function getThreadReplyAvatarCenterYPx() {
  return THREAD_REPLY_AVATAR_CENTER_Y_PX;
}

export function getThreadReplyDescendantRailStartYPx() {
  return (
    THREAD_REPLY_AVATAR_CENTER_Y_PX +
    THREAD_REPLY_AVATAR_RADIUS_PX +
    THREAD_REPLY_AVATAR_LINE_GAP_PX
  );
}

export function getThreadReplyConnectorLayout(depth: number) {
  const visibleDepth = clampVisibleDepth(depth);
  if (visibleDepth === 0) {
    return null;
  }

  const parentOffsetPx = getThreadReplyAvatarCenterPx(visibleDepth - 1);
  const childOffsetPx = getThreadReplyAvatarCenterPx(visibleDepth);
  const childEdgeOffsetPx =
    childOffsetPx -
    THREAD_REPLY_AVATAR_RADIUS_PX -
    THREAD_REPLY_AVATAR_LINE_GAP_PX;

  return {
    childOffsetPx,
    heightPx: THREAD_REPLY_AVATAR_CENTER_Y_PX,
    parentOffsetPx,
    widthPx: Math.max(0, childEdgeOffsetPx - parentOffsetPx),
  };
}
