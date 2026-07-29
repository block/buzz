import { ChannelNotifyDialog } from "./ChannelNotifyDialog";
import { NonMemberMentionDialog } from "./NonMemberMentionDialog";
import type { UseMentionSendFlowResult } from "./useMentionSendFlow";

type ComposerMentionDialogsProps = {
  /** Channel member count, used to size the `@channel` prompt. */
  memberCount: number;
  sendFlow: UseMentionSendFlowResult;
};

/**
 * The prompts the send flow can interpose before a message goes out, in the
 * order the flow raises them: confirm a channel-wide mention, then decide what
 * to do about mentioned non-members. At most one is open at a time.
 */
export function ComposerMentionDialogs({
  memberCount,
  sendFlow,
}: ComposerMentionDialogsProps) {
  return (
    <>
      <ChannelNotifyDialog
        isSendPending={sendFlow.isPreparingMentionSend}
        memberCount={memberCount}
        mode={sendFlow.pendingChannelNotifyMode}
        onCancel={sendFlow.dismissChannelNotifyPrompt}
        onConfirm={sendFlow.confirmChannelNotifySend}
      />

      <NonMemberMentionDialog
        error={sendFlow.nonMemberPromptError}
        isInvitePending={sendFlow.isInvitePending}
        names={sendFlow.pendingNonMemberNames}
        onDismiss={sendFlow.dismissNonMemberPrompt}
        onDoNothing={sendFlow.sendWithoutInviting}
        onInvite={sendFlow.inviteNonMembers}
        open={sendFlow.pendingNonMemberSend !== null}
      />
    </>
  );
}
