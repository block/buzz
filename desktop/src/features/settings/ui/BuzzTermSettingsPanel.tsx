import * as React from "react";
import { homeDir } from "@tauri-apps/api/path";

import { expandTilde } from "@/features/communities/communityStorage";
import {
  DEFAULT_TERM_CWD,
  DEFAULT_TERM_FONT_SIZE,
  DEFAULT_TERM_OPEN_MODE,
  DEFAULT_TERM_SCROLLBACK,
  DEFAULT_TERM_SESSION_HOST,
  DEFAULT_TERM_SESSION_NAME,
  DEFAULT_TERM_SHELL,
  TERM_FONT_SIZES,
  type TermFontSize,
  type TermOpenMode,
  type TermSessionHost,
  getTermPreferences,
  setTermCwd,
  setTermFontSize,
  setTermOpenMode,
  setTermScrollback,
  setTermSessionHost,
  setTermSessionName,
  setTermShell,
  useTermPreferences,
} from "@/features/terminal/termPreferences";
import { isHerdrAvailable } from "@/features/terminal/herdrHost";
import { Alert, AlertDescription, AlertTitle } from "@/shared/ui/alert";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import { SegmentedControl } from "@/shared/ui/segmented-control";
import { cn } from "@/shared/lib/cn";

import {
  SettingsOptionGroup,
  SettingsOptionGroupList,
  SettingsOptionRow,
} from "./SettingsOptionGroup";
import { RemoteTerminalCard } from "./RemoteAccessSettingsCard";
import { SettingsSectionHeader } from "./SettingsSectionHeader";

const SESSION_HOST_OPTIONS = [
  { value: "buzz-term" as const, label: "Buzz Term tabs" },
  { value: "herdr" as const, label: "herdr" },
];

/**
 * Settings → Buzz Term: cwd, shell, scrollback, font size, channel open mode,
 * session host (Buzz Term tabs | herdr). herdr-only prefs appear under the host
 * radio when herdr is selected.
 */
export function BuzzTermSettingsPanel() {
  const prefs = useTermPreferences();
  const [cwdDraft, setCwdDraft] = React.useState(prefs.cwd);
  const [shellDraft, setShellDraft] = React.useState(prefs.shell);
  const [scrollDraft, setScrollDraft] = React.useState(String(prefs.scrollback));
  const [sessionNameDraft, setSessionNameDraft] = React.useState(prefs.sessionName);
  const [cwdError, setCwdError] = React.useState<string | null>(null);
  const [herdrAvailable, setHerdrAvailable] = React.useState<boolean | null>(
    null,
  );

  React.useEffect(() => {
    setCwdDraft(prefs.cwd);
    setShellDraft(prefs.shell);
    setScrollDraft(String(prefs.scrollback));
    setSessionNameDraft(prefs.sessionName);
  }, [prefs.cwd, prefs.shell, prefs.scrollback, prefs.sessionName]);

  React.useEffect(() => {
    if (prefs.sessionHost !== "herdr") {
      setHerdrAvailable(null);
      return;
    }
    let cancelled = false;
    void isHerdrAvailable().then((ok) => {
      if (!cancelled) setHerdrAvailable(ok);
    });
    return () => {
      cancelled = true;
    };
  }, [prefs.sessionHost]);

  const commitCwd = React.useCallback(async (raw: string) => {
    setCwdError(null);
    const trimmed = raw.trim();
    if (!trimmed || trimmed === "~") {
      setTermCwd(DEFAULT_TERM_CWD);
      setCwdDraft(DEFAULT_TERM_CWD);
      return;
    }
    try {
      const expanded = (await expandTilde(trimmed)) ?? trimmed;
      const home = (await homeDir()).replace(/\/$/, "");
      if (expanded.replace(/\/$/, "") === home) {
        setTermCwd(DEFAULT_TERM_CWD);
        setCwdDraft(DEFAULT_TERM_CWD);
      } else {
        setTermCwd(expanded);
        setCwdDraft(expanded);
      }
    } catch {
      setCwdError("Could not resolve that path.");
    }
  }, []);

  const commitShell = React.useCallback((raw: string) => {
    setTermShell(raw);
    setShellDraft(raw.trim());
  }, []);

  const commitScrollback = React.useCallback((raw: string) => {
    setTermScrollback(raw);
    setScrollDraft(String(getTermPreferences().scrollback));
  }, []);

  const commitSessionName = React.useCallback((raw: string) => {
    setTermSessionName(raw);
    setSessionNameDraft(getTermPreferences().sessionName);
  }, []);

  return (
    <section className="min-w-0 space-y-8" data-testid="buzz-term-settings">
      <SettingsSectionHeader
        description="Preferences for the in-app terminal."
        title="Buzz Term"
      />
      <SettingsOptionGroupList>
        <RemoteTerminalCard />
        <SettingsOptionGroup
          data-testid="buzz-term-session-card"
          title="Sessions"
        >
          <SettingsOptionRow data-testid="buzz-term-cwd-row">
            <div className="min-w-0 flex-1 space-y-2">
              <label
                className="text-sm font-medium"
                htmlFor="buzz-term-session-cwd"
              >
                New sessions start in
              </label>
              <p
                className="text-sm font-normal text-muted-foreground/70"
                data-settings-subcopy
              >
                Leave blank for the login-shell default (usually your home
                folder). An absolute path overrides. A Term Session handoff
                card still wins for that tab.
              </p>
              <div className="flex flex-wrap items-center gap-2 pt-1">
                <Input
                  autoComplete="off"
                  className="max-w-md font-mono text-sm"
                  data-testid="buzz-term-session-cwd-input"
                  id="buzz-term-session-cwd"
                  onBlur={() => void commitCwd(cwdDraft)}
                  onChange={(event) => setCwdDraft(event.target.value)}
                  onKeyDown={(event) => {
                    if (event.key === "Enter") {
                      event.preventDefault();
                      void commitCwd(cwdDraft);
                    }
                  }}
                  placeholder="(home)"
                  spellCheck={false}
                  value={cwdDraft}
                />
                <Button
                  data-testid="buzz-term-session-cwd-reset"
                  onClick={() => void commitCwd(DEFAULT_TERM_CWD)}
                  size="sm"
                  type="button"
                  variant="outline"
                >
                  Clear
                </Button>
              </div>
              {cwdError ? (
                <p className="text-xs text-destructive">{cwdError}</p>
              ) : null}
            </div>
          </SettingsOptionRow>

          <SettingsOptionRow data-testid="buzz-term-shell-row">
            <div className="min-w-0 flex-1 space-y-2">
              <label
                className="text-sm font-medium"
                htmlFor="buzz-term-shell"
              >
                Default shell
              </label>
              <p
                className="text-sm font-normal text-muted-foreground/70"
                data-settings-subcopy
              >
                Leave blank to use the system shell (<span className="font-mono">$SHELL</span>).
                Optional absolute path override (must be an executable file).
                Applies to Buzz Term tabs (and herdr fallback shells).
              </p>
              <div className="flex flex-wrap items-center gap-2 pt-1">
                <Input
                  autoComplete="off"
                  className="max-w-md font-mono text-sm"
                  data-testid="buzz-term-shell-input"
                  id="buzz-term-shell"
                  onBlur={() => commitShell(shellDraft)}
                  onChange={(event) => setShellDraft(event.target.value)}
                  onKeyDown={(event) => {
                    if (event.key === "Enter") {
                      event.preventDefault();
                      commitShell(shellDraft);
                    }
                  }}
                  placeholder="$SHELL"
                  spellCheck={false}
                  value={shellDraft}
                />
                <Button
                  data-testid="buzz-term-shell-reset"
                  onClick={() => commitShell(DEFAULT_TERM_SHELL)}
                  size="sm"
                  type="button"
                  variant="outline"
                >
                  Clear
                </Button>
              </div>
            </div>
          </SettingsOptionRow>

          <SettingsOptionRow data-testid="buzz-term-scrollback-row">
            <div className="min-w-0 flex-1 space-y-2">
              <label
                className="text-sm font-medium"
                htmlFor="buzz-term-scrollback"
              >
                Scrollback lines
              </label>
              <p
                className="text-sm font-normal text-muted-foreground/70"
                data-settings-subcopy
              >
                History depth for mouse-wheel scrollback. Default{" "}
                {DEFAULT_TERM_SCROLLBACK.toLocaleString()}. Applies to new
                sessions.
              </p>
              <Input
                className="max-w-[10rem] font-mono text-sm"
                data-testid="buzz-term-scrollback-input"
                id="buzz-term-scrollback"
                inputMode="numeric"
                onBlur={() => commitScrollback(scrollDraft)}
                onChange={(event) => setScrollDraft(event.target.value)}
                onKeyDown={(event) => {
                  if (event.key === "Enter") {
                    event.preventDefault();
                    commitScrollback(scrollDraft);
                  }
                }}
                value={scrollDraft}
              />
            </div>
          </SettingsOptionRow>
        </SettingsOptionGroup>

        <SettingsOptionGroup
          data-testid="buzz-term-appearance-card"
          title="Appearance"
        >
          <SettingsOptionRow data-testid="buzz-term-font-size-row">
            <div className="min-w-0">
              <p className="text-sm font-medium">Font size</p>
              <p
                className="text-sm font-normal text-muted-foreground/70"
                data-settings-subcopy
              >
                Term canvas only (font family comes later). Default{" "}
                {DEFAULT_TERM_FONT_SIZE}px.
              </p>
            </div>
            <div className="flex flex-wrap justify-end gap-1">
              {TERM_FONT_SIZES.map((size) => {
                const selected = prefs.fontSize === size;
                return (
                  <button
                    aria-pressed={selected}
                    className={cn(
                      "h-9 min-w-9 rounded-lg border px-2 text-sm font-medium transition-colors",
                      selected
                        ? "border-primary bg-primary text-primary-foreground"
                        : "border-border bg-background hover:bg-muted",
                    )}
                    data-testid={`buzz-term-font-size-${size}`}
                    key={size}
                    onClick={() => setTermFontSize(size as TermFontSize)}
                    type="button"
                  >
                    {size}
                  </button>
                );
              })}
            </div>
          </SettingsOptionRow>
        </SettingsOptionGroup>

        <SettingsOptionGroup
          data-testid="buzz-term-open-card"
          title="Open behavior"
        >
          <SettingsOptionRow data-testid="buzz-term-open-mode-row">
            <div className="min-w-0">
              <p className="text-sm font-medium">Channel / ⌘J</p>
              <p
                className="text-sm font-normal text-muted-foreground/70"
                data-settings-subcopy
              >
                How Term opens from the channel header or ⌘J. The left-nav Buzz
                Term control stays maximized with all tabs. Default{" "}
                {DEFAULT_TERM_OPEN_MODE}.
              </p>
            </div>
            <div className="flex flex-wrap justify-end gap-1">
              {(
                [
                  ["docked", "Docked"],
                  ["maximized", "Maximized"],
                ] as const
              ).map(([value, label]) => {
                const selected = prefs.openMode === value;
                return (
                  <button
                    aria-pressed={selected}
                    className={cn(
                      "h-9 rounded-lg border px-3 text-sm font-medium transition-colors",
                      selected
                        ? "border-primary bg-primary text-primary-foreground"
                        : "border-border bg-background hover:bg-muted",
                    )}
                    data-testid={`buzz-term-open-mode-${value}`}
                    key={value}
                    onClick={() => setTermOpenMode(value as TermOpenMode)}
                    type="button"
                  >
                    {label}
                  </button>
                );
              })}
            </div>
          </SettingsOptionRow>
        </SettingsOptionGroup>

        <SettingsOptionGroup
          data-testid="buzz-term-host-card"
          title="Session host"
        >
          <SettingsOptionRow data-testid="buzz-term-session-host-row">
            <div className="min-w-0">
              <p className="text-sm font-medium">New sessions open in</p>
              <p
                className="text-sm font-normal text-muted-foreground/70"
                data-settings-subcopy
              >
                Buzz Term tabs spawn an in-app shell. herdr attaches in-app to
                the named session (plain open); Term Session card Open creates a
                focused herdr workspace. Falls back to Buzz Term when herdr is
                missing. Default{" "}
                {DEFAULT_TERM_SESSION_HOST === "herdr" ? "herdr" : "Buzz Term tabs"}.
              </p>
            </div>
            <SegmentedControl
              legend="Session host"
              onValueChange={(value) =>
                setTermSessionHost(value as TermSessionHost)
              }
              optionTestIdPrefix="buzz-term-session-host"
              options={SESSION_HOST_OPTIONS}
              size="wide"
              testId="buzz-term-session-host-control"
              value={prefs.sessionHost}
            />
          </SettingsOptionRow>

          {prefs.sessionHost === "herdr" ? (
            <>
              <SettingsOptionRow data-testid="buzz-term-herdr-install-warning-row">
                <Alert
                  className="w-full"
                  data-testid="buzz-term-herdr-install-warning"
                  variant={herdrAvailable === false ? "destructive" : "default"}
                >
                  <AlertTitle>herdr must be installed</AlertTitle>
                  <AlertDescription>
                    {herdrAvailable === false ? (
                      <>
                        herdr was not detected on this machine. Install herdr
                        (CLI on <span className="font-mono">PATH</span> or{" "}
                        <span className="font-mono">~/.local/bin/herdr</span>)
                        and run <span className="font-mono">herdr session attach {prefs.sessionName || DEFAULT_TERM_SESSION_NAME}</span>{" "}
                        before opening Term sessions here. Buzz falls back to an
                        in-app tab while herdr is missing.
                      </>
                    ) : herdrAvailable === true ? (
                      <>
                        herdr is available. Buzz Term shows herdr in-app;
                        plain open attaches to the named session below (default{" "}
                        <span className="font-mono">{DEFAULT_TERM_SESSION_NAME}</span>
                        ); card Open creates a workspace.
                      </>
                    ) : (
                      <>
                        Checking for herdr… Named sessions require the herdr CLI
                        on this machine.
                      </>
                    )}
                  </AlertDescription>
                </Alert>
              </SettingsOptionRow>

              <SettingsOptionRow data-testid="buzz-term-session-name-row">
                <div className="min-w-0 flex-1 space-y-2">
                  <label
                    className="text-sm font-medium"
                    htmlFor="buzz-term-session-name"
                  >
                    Session name
                  </label>
                  <p
                    className="text-sm font-normal text-muted-foreground/70"
                    data-settings-subcopy
                  >
                    herdr session for attach/create/workspace/handoff (
                    <span className="font-mono">herdr session attach …</span>
                    ). Default{" "}
                    <span className="font-mono">{DEFAULT_TERM_SESSION_NAME}</span>.
                    Clear to use herdr&apos;s unnamed default session.
                  </p>
                  <div className="flex flex-wrap items-center gap-2 pt-1">
                    <Input
                      autoComplete="off"
                      className="max-w-md font-mono text-sm"
                      data-testid="buzz-term-session-name-input"
                      id="buzz-term-session-name"
                      onBlur={() => commitSessionName(sessionNameDraft)}
                      onChange={(event) =>
                        setSessionNameDraft(event.target.value)
                      }
                      onKeyDown={(event) => {
                        if (event.key === "Enter") {
                          event.preventDefault();
                          commitSessionName(sessionNameDraft);
                        }
                      }}
                      placeholder={DEFAULT_TERM_SESSION_NAME}
                      spellCheck={false}
                      value={sessionNameDraft}
                    />
                    <Button
                      data-testid="buzz-term-session-name-reset"
                      onClick={() => commitSessionName("")}
                      size="sm"
                      type="button"
                      variant="outline"
                    >
                      Clear
                    </Button>
                    <Button
                      data-testid="buzz-term-session-name-default"
                      onClick={() =>
                        commitSessionName(DEFAULT_TERM_SESSION_NAME)
                      }
                      size="sm"
                      type="button"
                      variant="outline"
                    >
                      Use {DEFAULT_TERM_SESSION_NAME}
                    </Button>
                  </div>
                </div>
              </SettingsOptionRow>
            </>
          ) : null}
        </SettingsOptionGroup>
      </SettingsOptionGroupList>
    </section>
  );
}
