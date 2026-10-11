// Web/mobile Browsers page. Same layout and components as the desktop Browsers screen (PageHeader, Add dialog, bordered list,
// thumbnail + title + Live/Cold chips + address, robot toggle with Off/Observe/Drive chips, per-row actions, empty state),
// but backed by the relay bridge: rows are the desktop's browsers, Add/Remove/Screenshot/robot are signed commands, and a
// screenshot replaces the live webview (there is none on web).
import { Bot, Camera, Globe, Plus, Trash2 } from "lucide-react";
import * as React from "react";

import { ModeChip } from "@/features/browser-agent/ui/BrowserAgentChrome";
import { useChannelsQuery } from "@/features/channels/hooks";
import type { BrowserSessionInfo } from "@/features/bridge/protocol";
import type { CommandBody, CommandType } from "@/features/bridge/protocol";
import { useBridgeAgents } from "@/features/bridge/useBridgeAgents";
import { useIdentityQuery } from "@/shared/api/hooks";
import { Badge } from "@/shared/ui/badge";
import { Button } from "@/shared/ui/button";
import { PageHeader } from "@/shared/ui/PageHeader";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";

import { browserPreviewFaviconHost } from "../lib/browserPreview";
import { jobPhase, phaseText, type JobPhase } from "../lib/browserCommandJob";
import { browserRowUrlLabel } from "../lib/browserRows";
import { AddBrowserDialog } from "./AddBrowserDialog";
import { BrowserRowThumb } from "./BrowserRowPreview";
import { BrowserScreenshotViewer } from "./BrowserScreenshotViewer";

type Job = {
  label: string;
  run: () => Promise<string>;
  id: string | null;
  sentAtMs: number | null;
  acceptedAtMs: number | null;
  error: string | null;
};

const selectClass = "h-7 rounded-md border border-input bg-background px-2 text-xs";

export function WebBrowsersScreen() {
  const pubkey = useIdentityQuery().data?.pubkey;
  const b = useBridgeAgents(pubkey);
  const commander = b.commander;
  const channels = (useChannelsQuery().data ?? []).filter((c) => c.channelType !== "dm");
  const [addOpen, setAddOpen] = React.useState(false);
  const [job, setJob] = React.useState<Job | null>(null);
  const [, tick] = React.useReducer((x: number) => x + 1, 0);
  const [viewer, setViewer] = React.useState<string | null>(null);
  const [pendingRemove, setPendingRemove] = React.useState<string | null>(null);
  const [robotOpen, setRobotOpen] = React.useState<string | null>(null);
  const [robot, setRobot] = React.useState<{ agent: string; channel: string; mode: "off" | "observe" | "drive" }>({ agent: "", channel: "", mode: "observe" });

  React.useEffect(() => {
    const t = window.setInterval(tick, 500);
    return () => window.clearInterval(t);
  }, []);
  React.useEffect(() => (commander ? commander.subscribe(tick) : undefined), [commander]);

  const online = b.presence === "online";
  const agentsAllowed = b.heartbeat?.remote?.agents === true && !!commander?.supports("browser.agent");
  const sessions = b.browserSessions;

  // Remember when the desktop first acknowledged (for the "accepted" timeout).
  const result = job?.id ? b.results.get(job.id) : undefined;
  React.useEffect(() => {
    if (job && result?.status === "accepted" && job.acceptedAtMs === null) setJob({ ...job, acceptedAtMs: Date.now() });
  }, [job, result?.status]);
  const phase: JobPhase | null = job
    ? jobPhase({ sentAtMs: job.sentAtMs, acceptedAtMs: job.acceptedAtMs, result: result?.status ?? null, nowMs: Date.now() })
    : null;
  // Successful jobs clear themselves.
  React.useEffect(() => {
    if (phase !== "done") return;
    const t = window.setTimeout(() => setJob(null), 4000);
    return () => window.clearTimeout(t);
  }, [phase]);

  function start(label: string, send: () => Promise<string>) {
    const j: Job = { label, run: send, id: null, sentAtMs: null, acceptedAtMs: null, error: null };
    setJob(j);
    send().then(
      (id) => setJob((cur) => (cur === j ? { ...j, id, sentAtMs: Date.now() } : cur)),
      (e) => setJob((cur) => (cur === j ? { ...j, error: e instanceof Error ? e.message : String(e) } : cur)),
    );
  }
  const send = (type: CommandType, body: CommandBody, label: string, confirmed = true) => () =>
    (commander as NonNullable<typeof commander>).send(type, body, { confirmed, label });

  const takeShot = (s: BrowserSessionInfo) =>
    start(`Screenshot ${s.host}`, send("browser.screenshot", { sessionId: s.sessionId }, `Screenshot ${s.host}`, false));
  const failedToSend = !!job?.error;
  const text = job
    ? failedToSend
      ? `${job.label}: ${job.error}`
      : phaseText(phase ?? "sending", job.label, result?.message)
    : null;
  const busyJob = !!job && !failedToSend && phase !== null && phase !== "done" && phase !== "failed" && phase !== "timeout";

  const empty = sessions.length === 0;
  return (
    <div className="flex-1 overflow-y-auto overflow-x-hidden overscroll-contain px-4 py-7 sm:px-6 sm:py-8" data-testid="web-browsers-page">
      <div className="mx-auto w-full max-w-6xl space-y-8" data-testid="browsers-page-content">
        <PageHeader
          action={
            <Button data-testid="browsers-add" disabled={!commander || !online || !commander.supports("browser.open")} onClick={() => setAddOpen(true)} size="sm" type="button">
              <Plus className="mr-1 h-4 w-4" />
              Add
            </Button>
          }
          description="Browsers on your desktop, shown here over the relay. Add, remove and screenshot them; give an agent Observe or Drive."
          title="Browsers"
        />

        {b.available && b.presence === "offline" ? (
          <p className="rounded-lg border border-border/70 px-4 py-3 text-sm" data-testid="web-browsers-offline">
            Your desktop is offline, so its browsers can't be shown or changed. Start the Buzz desktop app.
          </p>
        ) : null}
        {b.available && commander && online && !commander.supports("browser.open") ? (
          <p className="rounded-lg border border-border/70 px-4 py-3 text-sm">Update the desktop app to manage browsers from here.</p>
        ) : null}

        {text ? (
          <div
            aria-live="polite"
            className={`flex flex-wrap items-center gap-2 rounded-lg border px-4 py-3 text-sm ${phase === "failed" || phase === "timeout" || failedToSend ? "border-destructive/50 text-destructive" : "border-border/70"}`}
            data-phase={failedToSend ? "failed" : phase}
            data-testid="browser-job"
          >
            <span className="min-w-0 flex-1" data-testid="browser-job-text">{text}</span>
            {phase === "timeout" || phase === "failed" || failedToSend ? (
              <>
                <Button data-testid="browser-job-retry" onClick={() => job && start(job.label, job.run)} size="xs" type="button" variant="secondary">Retry</Button>
                <Button data-testid="browser-job-cancel" onClick={() => setJob(null)} size="xs" type="button" variant="ghost">Cancel</Button>
              </>
            ) : busyJob ? (
              <Button data-testid="browser-job-cancel" onClick={() => setJob(null)} size="xs" type="button" variant="ghost">Dismiss</Button>
            ) : null}
          </div>
        ) : null}

        {empty ? (
          <div className="rounded-lg border border-dashed border-border/70 px-6 py-12 text-center" data-testid="browsers-empty">
            <Globe className="mx-auto h-8 w-8 text-muted-foreground" />
            <p className="mt-3 text-sm font-medium">No browsers open yet</p>
            <p className="mt-1 text-sm text-muted-foreground">
              Use Add to open a site on your desktop — then grant an agent Observe or Drive from the Bot toggle.
            </p>
            <Button className="mt-4" data-testid="browsers-empty-add" disabled={!commander || !online} onClick={() => setAddOpen(true)} size="sm" type="button">
              <Plus className="mr-1 h-4 w-4" />
              Add browser
            </Button>
          </div>
        ) : (
          <ul className="divide-y divide-border/60 rounded-lg border border-border/70" data-testid="browser-panel">
            {sessions.map((s: BrowserSessionInfo) => {
              const shot = b.shots.get(s.sessionId);
              const open = robotOpen === s.sessionId;
              return (
                <li className="flex flex-col gap-2 px-4 py-3" data-testid="br-session" key={s.sessionId}>
                  <div className="flex min-w-0 flex-col gap-2">
                    <div className="flex min-w-0 flex-wrap items-start gap-3">
                      <BrowserRowThumb
                        host={s.url ? browserPreviewFaviconHost(s.url) : null}
                        onOpen={() => setViewer(s.sessionId)}
                        sid={s.sessionId}
                        src={shot?.src ?? null}
                        title={s.title || s.host}
                      />
                      <div className="min-w-0 flex-1 basis-40 text-left">
                        <div className="flex min-w-0 flex-wrap items-center gap-2">
                          <span className="truncate text-sm font-medium">{s.title || s.host}</span>
                          {s.status === "cold" ? (
                            <Badge className="whitespace-nowrap normal-case tracking-normal" data-testid="br-cold" title="Webview not started — a screenshot starts it" variant="secondary">Cold</Badge>
                          ) : null}
                          {s.status === "live" ? (
                            <Badge className="whitespace-nowrap normal-case tracking-normal" data-testid="br-live" variant="outline">Live</Badge>
                          ) : null}
                          <Badge className="whitespace-nowrap normal-case tracking-normal" variant="outline">{s.origin === "web" ? "Added from web" : "On desktop"}</Badge>
                          {s.agent ? (
                            <span className="truncate rounded-full bg-muted px-2 py-0.5 text-2xs text-muted-foreground" data-testid="br-agent-chip">
                              {s.agent.mode === "drive" ? "Drive" : "Observe"} · {s.agent.name}
                            </span>
                          ) : null}
                        </div>
                        <p className="mt-0.5 truncate text-2xs text-muted-foreground">
                          {s.restricted ? "Private address (not shared)" : browserRowUrlLabel(s.url ?? s.host)}
                        </p>
                        {shot ? (
                          <p className="mt-0.5 truncate text-2xs text-muted-foreground" data-testid="br-shot-time">
                            Screenshot taken {new Date(shot.ts * 1000).toLocaleTimeString()}
                          </p>
                        ) : null}
                      </div>
                      <div className="flex w-full shrink-0 flex-wrap items-center justify-start gap-2 sm:w-auto sm:justify-end">
                        {agentsAllowed ? (
                          <Tooltip>
                            <TooltipTrigger asChild>
                              <Button
                                aria-expanded={open}
                                aria-label="Observe & Drive"
                                className={s.agent ? "border-orange-500/50 text-orange-700 hover:bg-orange-500/10 dark:text-orange-400" : undefined}
                                data-testid="br-robot"
                                disabled={!online}
                                onClick={() => {
                                  setRobotOpen(open ? null : s.sessionId);
                                  setRobot({ agent: s.agent?.pubkey ?? "", channel: channels[0]?.id ?? "", mode: s.agent?.mode ?? "observe" });
                                }}
                                size="icon-xs"
                                type="button"
                                variant="outline"
                              >
                                <Bot />
                              </Button>
                            </TooltipTrigger>
                            <TooltipContent side="bottom">Observe & Drive</TooltipContent>
                          </Tooltip>
                        ) : null}
                        <Button
                          data-testid="br-shot"
                          disabled={!online || !commander || commander.shotBusy() || s.status === "cold"}
                          title={s.status === "cold" ? "Cold - start it on the desktop first" : undefined}
                          onClick={() => takeShot(s)}
                          size="xs"
                          type="button"
                          variant="secondary"
                        >
                          <Camera className="mr-1 h-3 w-3" />
                          {shot ? "New screenshot" : "Screenshot"}
                        </Button>
                        {pendingRemove === s.sessionId ? (
                          <>
                            <Button
                              aria-label="Confirm remove browser"
                              data-testid="br-confirm-yes"
                              onClick={() => {
                                setPendingRemove(null);
                                start(`Remove ${s.title || s.host}`, send("browser.close", { sessionId: s.sessionId }, `Remove ${s.host}`));
                              }}
                              size="xs"
                              type="button"
                              variant="destructive"
                            >
                              Confirm?
                            </Button>
                            <Button onClick={() => setPendingRemove(null)} size="xs" type="button" variant="ghost">Cancel</Button>
                          </>
                        ) : (
                          <Button aria-label="Remove browser" data-testid="br-remove" disabled={!online} onClick={() => setPendingRemove(s.sessionId)} size="xs" type="button" variant="ghost">
                            <Trash2 className="h-3 w-3 text-destructive" />
                            <span className="ml-1 text-destructive">Remove</span>
                          </Button>
                        )}
                      </div>
                    </div>
                    {agentsAllowed && open ? (
                      <div className="flex min-w-0 flex-wrap items-center gap-1 py-0.5" data-testid="br-robot-panel">
                        {(["off", "observe", "drive"] as const).map((m) => (
                          <ModeChip
                            active={robot.mode === m}
                            key={m}
                            label={m === "off" ? "Off" : m === "observe" ? "Observe" : "Drive"}
                            onClick={() => setRobot({ ...robot, mode: m })}
                            testId={`br-robot-mode-${m}`}
                          />
                        ))}
                        {robot.mode !== "off" ? (
                          <>
                            <select aria-label="Agent" className={selectClass} data-testid="br-robot-agent" onChange={(e) => setRobot({ ...robot, agent: e.target.value })} value={robot.agent}>
                              <option value="">Choose agent…</option>
                              {(b.snapshot?.agents ?? []).map((a) => (
                                <option key={a.pubkey} value={a.pubkey}>{a.name}</option>
                              ))}
                            </select>
                            <select aria-label="Channel" className={selectClass} data-testid="br-robot-channel" onChange={(e) => setRobot({ ...robot, channel: e.target.value })} value={robot.channel}>
                              {channels.map((c) => (
                                <option key={c.id} value={c.id}>{c.name}</option>
                              ))}
                            </select>
                          </>
                        ) : null}
                        <Button
                          data-testid="br-robot-apply"
                          disabled={robot.mode !== "off" && (!robot.agent || !robot.channel)}
                          onClick={() => {
                            const label = robot.mode === "off" ? `Stop agent on ${s.host}` : `${robot.mode === "drive" ? "Drive" : "Observe"} ${s.host}`;
                            start(label, send("browser.agent", robot.mode === "off" ? { sessionId: s.sessionId, mode: "off" } : { sessionId: s.sessionId, mode: robot.mode, agentPubkey: robot.agent, channelId: robot.channel }, label));
                            setRobotOpen(null);
                          }}
                          size="xs"
                          type="button"
                        >
                          {robot.mode === "drive" ? "Let the agent drive" : "Apply"}
                        </Button>
                        {robot.mode === "drive" ? <span className="text-2xs text-muted-foreground">The agent can click and type on your desktop browser.</span> : null}
                      </div>
                    ) : null}
                  </div>
                </li>
              );
            })}
          </ul>
        )}
      </div>
      {viewer && sessions.find((x) => x.sessionId === viewer) ? (
        (() => {
          const vs = sessions.find((x) => x.sessionId === viewer) as BrowserSessionInfo;
          const vshot = b.shots.get(vs.sessionId);
          const isShotJob = !!job && job.label.startsWith("Screenshot");
          return (
            <BrowserScreenshotViewer
              busy={busyJob || !!commander?.shotBusy()}
              disabled={!online || !commander || vs.status === "cold"}
              onClose={() => setViewer(null)}
              onNew={() => takeShot(vs)}
              src={vshot?.src ?? null}
              statusText={isShotJob ? text : null}
              statusTone={phase === "failed" || phase === "timeout" || failedToSend ? "error" : "info"}
              takenAt={vshot?.ts ?? null}
              title={vs.title || vs.host}
              url={vs.restricted ? "Private address (not shared)" : vs.url ?? vs.host}
            />
          );
        })()
      ) : null}
      <AddBrowserDialog
        onOpenChange={setAddOpen}
        onSubmit={(card) => start(`Add ${card.url}`, send("browser.open", { url: card.url }, `Add ${card.url.slice(0, 40)}`))}
        open={addOpen}
      />
    </div>
  );
}
