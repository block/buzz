import { ChannelCanvas } from "./ChannelCanvas";

type KeyedChannelCanvasProps = {
  channelId: string | null;
  canEdit: boolean;
  isArchived: boolean;
};

/**
 * Renders {@link ChannelCanvas} keyed by `channelId` so a channel switch
 * remounts the subtree and drops all of its local edit state (edit mode, draft,
 * base revision, history selection, and the set-canvas mutation instance) in
 * one move. Without the key, `ChannelManagementSheet` stays mounted across a
 * channel switch and a draft typed against a canvas-less channel could publish
 * as another channel's canvas under the stale `none` precondition.
 *
 * The key lives here — not inline at the call site — so the reset boundary is a
 * single production seam the regression test consumes directly.
 */
export function KeyedChannelCanvas({
  channelId,
  canEdit,
  isArchived,
}: KeyedChannelCanvasProps) {
  return (
    <ChannelCanvas
      key={channelId ?? "none"}
      canEdit={canEdit}
      channelId={channelId}
      isArchived={isArchived}
    />
  );
}
