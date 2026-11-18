export type GuardedNavigation =
  | {
      kind: "history";
      direction: "back" | "forward";
    }
  | {
      kind: "route";
      href: string;
    }
  | {
      kind: "channel-message";
      channelId: string;
      messageId: string;
      threadRootId: string | null;
    }
  | {
      kind: "forum-post";
      channelId: string;
      postId: string;
      replyId: string | null;
    };

type NavigationGuard = (target: GuardedNavigation) => boolean;

type GuardRegistration = {
  guard: NavigationGuard;
};

const activeGuards: GuardRegistration[] = [];

export function allowNavigation(target: GuardedNavigation): boolean {
  return activeGuards.at(-1)?.guard(target) ?? true;
}

export function traverseHistory(
  history: Pick<History, "back" | "forward">,
  direction: "back" | "forward",
): boolean {
  if (!allowNavigation({ kind: "history", direction })) {
    return false;
  }

  history[direction]();
  return true;
}

export function registerNavigationGuard(guard: NavigationGuard): () => void {
  const registration = { guard };
  activeGuards.push(registration);
  return () => {
    const index = activeGuards.lastIndexOf(registration);
    if (index >= 0) activeGuards.splice(index, 1);
  };
}
