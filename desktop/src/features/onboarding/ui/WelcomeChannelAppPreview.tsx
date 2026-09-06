import {
  ArrowUp,
  AtSign,
  Bot,
  ChevronLeft,
  ChevronRight,
  FolderKanban,
  Hash,
  Headphones,
  Home,
  Lock,
  MoreVertical,
  Paperclip,
  Plus,
  Search,
  Smile,
  TerminalSquare,
  Users,
} from "lucide-react";
import * as React from "react";

import { ChannelIntroBlock } from "@/features/messages/ui/ChannelIntroBlock";
import { ComposerDockBackdrop } from "@/features/messages/ui/ComposerDockBackdrop";
import { ProfileAvatar } from "@/features/profile/ui/ProfileAvatar";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { Dialog, DialogContent, DialogTitle } from "@/shared/ui/dialog";
import { StartupWindowDragRegion } from "@/shared/ui/StartupWindowDragRegion";
import {
  HARNESS_CONNECTION_OPTIONS,
  HarnessConnectionDetailPreview,
  HarnessConnectionMethodPreview,
  type HarnessConnectionMethod,
  HarnessConnectionPreview,
  runtimeNeedsOnboardingConnection,
} from "./HarnessConnectionStep";
import { OnboardingFooterProvider } from "./OnboardingFooter";
import { OnboardingPreviewLayoutProvider } from "./OnboardingPreviewShell";

const CHANNEL_ROW_CLASS =
  "flex h-8 w-full items-center gap-2 rounded-md px-2 text-left text-sm transition-colors hover:bg-black/[0.06]";
const WELCOME_TEAM = [
  { name: "Fizz", image: "/onboarding/starter-team/fizz.png" },
  { name: "Honey", image: "/onboarding/starter-team/honey.png" },
  { name: "Pollen", image: "/onboarding/starter-team/pollen.png" },
] as const;
type PreviewThemeStyle = React.CSSProperties & Record<`--${string}`, string>;

const PREVIEW_APP_THEME: PreviewThemeStyle = {
  colorScheme: "light",
  "--accent": "0 0% 94%",
  "--accent-foreground": "0 0% 10%",
  "--background": "0 0% 100%",
  "--border": "0 0% 87%",
  "--card": "0 0% 100%",
  "--card-foreground": "0 0% 10%",
  "--foreground": "0 0% 10%",
  "--input": "0 0% 84%",
  "--muted": "0 0% 94%",
  "--muted-foreground": "0 0% 42%",
  "--popover": "0 0% 100%",
  "--popover-foreground": "0 0% 10%",
  "--primary": "0 0% 10%",
  "--primary-foreground": "0 0% 100%",
  "--ring": "0 0% 20%",
  "--secondary": "0 0% 96%",
  "--secondary-foreground": "0 0% 10%",
  "--sidebar": "48 48% 88%",
  "--sidebar-accent": "0 0% 100%",
  "--sidebar-accent-foreground": "0 0% 10%",
  "--sidebar-background": "48 48% 88%",
  "--sidebar-border": "0 0% 60%",
  "--sidebar-foreground": "0 0% 18%",
  "--sidebar-ring": "0 0% 20%",
};

type WelcomeHarnessPage = "method" | "list" | "detail";

function WelcomeHarnessConnectionDialog({
  onConnected,
  onOpenChange,
  open,
}: {
  onConnected: () => void;
  onOpenChange: (open: boolean) => void;
  open: boolean;
}) {
  const [page, setPage] = React.useState<WelcomeHarnessPage>("method");
  const [method, setMethod] =
    React.useState<HarnessConnectionMethod>("subscription");
  const [selectedHarnessId, setSelectedHarnessId] =
    React.useState("buzz-agent");
  const [detailBackPage, setDetailBackPage] =
    React.useState<WelcomeHarnessPage>("list");
  const [installedIds, setInstalledIds] = React.useState(
    () =>
      new Set(
        HARNESS_CONNECTION_OPTIONS.filter(
          ({ runtime }) => runtime.availability === "available",
        ).map(({ runtime }) => runtime.id),
      ),
  );
  const selectedHarness =
    HARNESS_CONNECTION_OPTIONS.find(
      ({ runtime }) => runtime.id === selectedHarnessId,
    ) ?? HARNESS_CONNECTION_OPTIONS[0];

  React.useEffect(() => {
    if (!open) return;
    setPage("method");
    setMethod("subscription");
    setSelectedHarnessId("buzz-agent");
    setDetailBackPage("list");
  }, [open]);

  const completeConnection = React.useCallback(() => {
    onConnected();
    onOpenChange(false);
  }, [onConnected, onOpenChange]);

  let content: React.ReactNode;
  if (page === "method") {
    content = (
      <HarnessConnectionMethodPreview
        embedded
        onBack={() => onOpenChange(false)}
        onSelect={(nextMethod) => {
          setMethod(nextMethod);
          if (nextMethod === "api") {
            setSelectedHarnessId("buzz-agent");
            setDetailBackPage("method");
            setPage("detail");
          } else {
            setPage("list");
          }
        }}
        onSetUpLater={() => onOpenChange(false)}
        total={5}
      />
    );
  } else if (page === "list") {
    content = (
      <HarnessConnectionPreview
        embedded
        installedIds={installedIds}
        method={method}
        onBack={() => setPage("method")}
        onSelect={(option) => {
          setSelectedHarnessId(option.runtime.id);
          setDetailBackPage("list");
          if (
            installedIds.has(option.runtime.id) &&
            !runtimeNeedsOnboardingConnection(method, option.runtime.id)
          ) {
            completeConnection();
          } else {
            setPage("detail");
          }
        }}
        total={5}
      />
    );
  } else {
    content = (
      <HarnessConnectionDetailPreview
        embedded
        installed={installedIds.has(selectedHarness.runtime.id)}
        key={`${selectedHarness.runtime.id}-${method}`}
        lockMethod
        method={method}
        onBack={() => setPage(detailBackPage)}
        onCheckAgain={() => {
          setInstalledIds((current) =>
            new Set(current).add(selectedHarness.runtime.id),
          );
          if (
            !runtimeNeedsOnboardingConnection(
              method,
              selectedHarness.runtime.id,
            )
          ) {
            completeConnection();
          }
        }}
        onContinue={completeConnection}
        onMethodChange={setMethod}
        onUseDifferentHarness={
          method === "api" && selectedHarness.runtime.id === "buzz-agent"
            ? () => setPage("list")
            : undefined
        }
        option={selectedHarness}
        total={5}
      />
    );
  }

  const backAction =
    page === "method"
      ? undefined
      : {
          onClick: () => setPage(page === "detail" ? detailBackPage : "method"),
        };

  return (
    <Dialog onOpenChange={onOpenChange} open={open}>
      <DialogContent
        className="buzz-onboarding-neutral-theme !flex h-[min(41.5rem,calc(100dvh-3rem))] max-h-[calc(100dvh-2rem)] flex-col gap-0 overflow-hidden rounded-[2rem] bg-white p-12 text-left text-foreground sm:max-w-[38rem] [&_.buzz-onboarding-transition-content]:min-w-[32rem] [&_.buzz-onboarding-transition-content]:!text-left [&_h1+p]:!mx-0 [&_h1+p]:!mt-2 [&_h1+p]:!text-left [&_h1+p]:!text-base [&_h1+p]:!leading-6 [&_h1]:!text-left [&_h1]:!text-2xl [&_h1]:!leading-8 [&_h1]:!text-foreground"
        data-testid="welcome-preview-harness-dialog"
        style={PREVIEW_APP_THEME}
      >
        <DialogTitle className="sr-only">Connect AI provider</DialogTitle>
        <OnboardingPreviewLayoutProvider card>
          <OnboardingFooterProvider backAction={backAction} placement="card">
            <div className="flex min-h-0 min-w-[32rem] flex-1 flex-col">
              {content}
            </div>
          </OnboardingFooterProvider>
        </OnboardingPreviewLayoutProvider>
      </DialogContent>
    </Dialog>
  );
}

function PreviewAvatar({
  avatarUrl,
  label,
  size = "small",
}: {
  avatarUrl: string;
  label: string;
  size?: "small" | "message";
}) {
  const className =
    size === "message"
      ? "h-9 w-9 rounded-lg text-xs"
      : "h-8 w-8 rounded-lg text-xs";
  if (avatarUrl) {
    return (
      <ProfileAvatar
        avatarUrl={avatarUrl}
        className={className}
        label={label}
      />
    );
  }

  return (
    <span
      aria-label={label}
      className={cn(
        "flex shrink-0 items-center justify-center bg-black font-semibold text-white",
        className,
      )}
      role="img"
    >
      {label.trim().charAt(0).toUpperCase() || "Y"}
    </span>
  );
}

function PreviewSidebar({
  avatarUrl,
  channelName,
  communityName,
  profileName,
  onPreviewAction,
}: {
  avatarUrl: string;
  channelName: string;
  communityName: string;
  profileName: string;
  onPreviewAction: (action: string) => void;
}) {
  return (
    <aside className="flex w-[344px] shrink-0 flex-col pb-2 pr-2 pt-10 text-black/80">
      <div className="flex h-9 items-center gap-0.5 px-2">
        <Button
          aria-label="Go back"
          className="size-7 rounded-md text-black/35"
          disabled
          size="icon"
          variant="ghost"
        >
          <ChevronLeft className="size-4" />
        </Button>
        <Button
          aria-label="Go forward"
          className="size-7 rounded-md text-black/35"
          disabled
          size="icon"
          variant="ghost"
        >
          <ChevronRight className="size-4" />
        </Button>
      </div>

      <div className="px-2 pb-3 pt-1">
        <button
          className="flex h-9 w-full items-center gap-2 rounded-lg bg-white/25 px-3 text-sm text-black/45 transition-colors hover:bg-white/45"
          onClick={() => onPreviewAction("Search")}
          type="button"
        >
          <Search className="size-4" />
          <span className="flex-1 text-left">Search everything</span>
          <span className="text-xs">⌘K</span>
        </button>
      </div>

      <nav aria-label="Workspace" className="space-y-0.5 px-2">
        <button className={CHANNEL_ROW_CLASS} type="button">
          <Home className="size-4 text-black/50" />
          Home
        </button>
        <button className={CHANNEL_ROW_CLASS} type="button">
          <FolderKanban className="size-4 text-black/50" />
          Projects
        </button>
        <button className={CHANNEL_ROW_CLASS} type="button">
          <Bot className="size-4 text-black/50" />
          Agents
        </button>
      </nav>

      <div className="mt-5 flex items-center justify-between px-4 text-xs font-medium text-black/45">
        <span>Channels</span>
        <Plus className="size-3.5" />
      </div>
      <div className="mt-1 space-y-0.5 px-2">
        <button className={CHANNEL_ROW_CLASS} type="button">
          <Hash className="size-4 text-black/45" />
          general
        </button>
        <button className={CHANNEL_ROW_CLASS} type="button">
          <Hash className="size-4 text-black/45" />
          welcome-everyone
        </button>
        <button
          className={cn(CHANNEL_ROW_CLASS, "bg-black/[0.07] font-semibold")}
          type="button"
        >
          <Lock className="size-4 text-black/55" />
          {channelName}
        </button>
      </div>

      <button
        className="mt-auto flex items-center gap-2 rounded-xl px-2 py-2 text-left hover:bg-black/[0.05]"
        onClick={() => onPreviewAction("Profile")}
        type="button"
      >
        <PreviewAvatar avatarUrl={avatarUrl} label={profileName} />
        <div className="min-w-0 flex-1">
          <p className="truncate text-sm font-semibold text-black/85">
            {profileName}
          </p>
          <p className="truncate text-xs text-black/45">🐝 {communityName}</p>
        </div>
      </button>
    </aside>
  );
}

export function WelcomeChannelAppPreview({
  avatarUrl,
  communityName,
  displayName,
}: {
  avatarUrl: string;
  communityName: string;
  displayName: string;
}) {
  const [draft, setDraft] = React.useState("");
  const [messages, setMessages] = React.useState<string[]>([]);
  const [harnessDialogOpen, setHarnessDialogOpen] = React.useState(false);
  const [aiProviderConnected, setAiProviderConnected] = React.useState(false);
  const [activePreviewAction, setActivePreviewAction] = React.useState<
    string | null
  >(null);
  const channelName = "Welcome";
  const profileName = displayName.trim() || "Your profile";
  const submitDraft = (event: React.FormEvent) => {
    event.preventDefault();
    const message = draft.trim();
    if (!message) return;
    setMessages((current) => [...current, message]);
    setDraft("");
  };

  const intro = {
    actions: [
      {
        icon: <Hash aria-hidden className="size-4" />,
        label: "Browse channels",
        onClick: () => setActivePreviewAction("Browse channels"),
        testId: "welcome-intro-action-browse-channels",
      },
      {
        icon: <Plus aria-hidden className="size-4" />,
        label: "Create a channel",
        onClick: () => setActivePreviewAction("Create a channel"),
        testId: "welcome-intro-action-create-channel",
      },
      {
        icon: <Bot aria-hidden className="size-4" />,
        label: "Create an agent",
        onClick: () => setActivePreviewAction("Create an agent"),
        testId: "welcome-intro-action-create-agent",
      },
    ],
    channelKindLabel: "private welcome channel",
    channelName,
    description: null,
  };

  return (
    <div
      className="flex h-dvh w-full overflow-hidden bg-[linear-gradient(145deg,#f3ed8d_0%,#dce8bf_48%,#c8dde8_100%)] text-foreground"
      data-testid="onboarding-preview-community-home"
      style={PREVIEW_APP_THEME}
    >
      <StartupWindowDragRegion />
      <PreviewSidebar
        avatarUrl={avatarUrl}
        channelName={channelName}
        communityName={communityName}
        onPreviewAction={setActivePreviewAction}
        profileName={profileName}
      />

      <main className="relative z-10 m-2 ml-0 flex min-w-0 flex-1 flex-col overflow-hidden rounded-xl bg-white text-black shadow-[-1px_0_0_0_rgb(0_0_0_/_0.08)]">
        <header
          className="relative z-30 shrink-0 cursor-default select-none bg-white px-5 py-2"
          data-testid="chat-header"
          data-tauri-drag-region
        >
          <div className="flex h-9 min-w-0 items-center gap-2.5">
            <div className="flex min-w-0 flex-1 items-center gap-1">
              <Lock className="size-4 text-black/45" />
              <h1
                className="min-w-0 truncate text-base font-semibold leading-6 tracking-tight"
                data-testid="chat-title"
              >
                {channelName}
              </h1>
            </div>
            <div className="flex shrink-0 items-center gap-1.5">
              <Button
                aria-label="Open terminal"
                className="size-9 rounded-xl"
                size="icon"
                variant="outline"
              >
                <TerminalSquare className="size-4" />
              </Button>
              <Button
                aria-label="View members"
                className="h-9 gap-2 rounded-xl px-3"
                variant="outline"
              >
                <Users className="size-4" />
                <span>4</span>
              </Button>
              <Button
                aria-label="Start huddle"
                className="size-9 rounded-xl"
                size="icon"
                variant="outline"
              >
                <Headphones className="size-4" />
              </Button>
              <Button
                aria-label="More channel actions"
                className="size-9 rounded-xl"
                size="icon"
                variant="outline"
              >
                <MoreVertical className="size-4" />
              </Button>
            </div>
          </div>
        </header>

        <div className="min-h-0 flex-1 overflow-y-auto px-5 pb-48 pt-5">
          <div className="mx-auto w-full max-w-[920px]">
            <ChannelIntroBlock intro={intro} />

            <section
              className="mx-3 mt-7 max-w-[680px] border-t border-border/60 pt-6 text-left"
              data-testid="welcome-preview-community-message"
            >
              <p className="text-lg font-semibold">
                Welcome to {communityName}
              </p>
              <p className="mt-2 text-sm leading-6 text-muted-foreground">
                Everyone starts with a private channel to get settled. This one
                is yours. Use it to try out features, draft messages, or work
                privately with agents.
              </p>
            </section>

            {!aiProviderConnected ? (
              <section
                className="mx-3 mt-6 max-w-[680px] overflow-hidden rounded-2xl border border-border/70 bg-muted/35 text-left"
                data-testid="welcome-preview-agent-activation"
              >
                <div className="flex items-center gap-4 p-5">
                  <div className="flex shrink-0 -space-x-3">
                    {WELCOME_TEAM.map((agent) => (
                      <img
                        alt={agent.name}
                        className="size-14 rounded-2xl border-2 border-background bg-background object-contain"
                        key={agent.name}
                        src={agent.image}
                      />
                    ))}
                  </div>
                  <div className="min-w-0 flex-1">
                    <div className="flex items-center gap-2">
                      <p className="text-base font-semibold">
                        Bring your starter team online
                      </p>
                      <span className="rounded-full bg-foreground/[0.08] px-2 py-0.5 text-xs font-medium text-muted-foreground">
                        Not connected
                      </span>
                    </div>
                    <p className="mt-1.5 text-sm leading-5 text-muted-foreground">
                      Connect an AI provider to start Fizz, Honey, and Pollen.
                      They can help you learn Buzz and work through something
                      you’re building.
                    </p>
                  </div>
                </div>
                <div className="flex items-center justify-between border-t border-border/60 bg-background/45 px-5 py-3">
                  <p className="text-xs text-muted-foreground">
                    You can change providers or agents later.
                  </p>
                  <Button
                    className="h-9 rounded-full px-5"
                    onClick={() => setHarnessDialogOpen(true)}
                    type="button"
                  >
                    Connect AI provider
                  </Button>
                </div>
              </section>
            ) : null}

            {messages.map((message, index) => (
              <div
                className="mt-6 flex items-start gap-3 px-3 text-left"
                key={`${message}-${index.toString()}`}
              >
                <PreviewAvatar
                  avatarUrl={avatarUrl}
                  label={profileName}
                  size="message"
                />
                <div>
                  <div className="flex items-baseline gap-2">
                    <p className="text-sm font-semibold">{profileName}</p>
                    <p className="text-xs text-muted-foreground">Now</p>
                  </div>
                  <p className="mt-1 text-sm leading-5">{message}</p>
                </div>
              </div>
            ))}
          </div>
        </div>

        <div className="pointer-events-none absolute inset-x-0 bottom-0 z-40 isolate before:absolute before:inset-x-0 before:bottom-0 before:-z-10 before:h-24 before:bg-gradient-to-b before:from-transparent before:to-white before:content-[''] after:absolute after:inset-x-0 after:bottom-0 after:-z-10 after:h-12 after:bg-white after:content-['']">
          <div className="composer-dock composer-overlay-corner-masks relative pointer-events-auto">
            <ComposerDockBackdrop gutterClassName="inset-x-4" />
            <form
              className="relative z-10 mx-4 mb-3 rounded-2xl border border-black/10 bg-white/95 px-4 pb-3 pt-3 backdrop-blur-md"
              data-testid="message-composer"
              onSubmit={submitDraft}
            >
              <textarea
                aria-label={`Message ${channelName}`}
                className="min-h-11 w-full resize-none bg-transparent text-sm leading-5 outline-none placeholder:text-black/45"
                data-testid="welcome-preview-composer-input"
                onChange={(event) => setDraft(event.target.value)}
                placeholder={`Message #${channelName}`}
                rows={2}
                value={draft}
              />
              <div className="flex items-center gap-1">
                {[AtSign, Paperclip, Smile].map((Icon) => (
                  <Button
                    className="size-7 text-black/55"
                    key={Icon.displayName ?? Icon.name}
                    size="icon"
                    type="button"
                    variant="ghost"
                  >
                    <Icon className="size-4" />
                  </Button>
                ))}
                <Button
                  aria-label="Formatting"
                  className="size-7 text-xs font-semibold text-black/55"
                  size="icon"
                  type="button"
                  variant="ghost"
                >
                  Aa
                </Button>
                <Button
                  aria-label="Send message"
                  className="ml-auto size-9 rounded-full"
                  disabled={!draft.trim()}
                  size="icon"
                  type="submit"
                >
                  <ArrowUp className="size-4" />
                </Button>
              </div>
            </form>
          </div>
        </div>

        {activePreviewAction ? (
          <div
            className="absolute right-5 top-16 z-50 rounded-lg border border-black/10 bg-white px-3 py-2 text-xs shadow-lg"
            role="status"
          >
            {activePreviewAction}
          </div>
        ) : null}
      </main>
      <WelcomeHarnessConnectionDialog
        onConnected={() => setAiProviderConnected(true)}
        onOpenChange={setHarnessDialogOpen}
        open={harnessDialogOpen}
      />
    </div>
  );
}
