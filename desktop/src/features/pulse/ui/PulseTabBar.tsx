import { Check, Filter, Search } from "lucide-react";

import type { PulseTab } from "@/features/pulse/ui/PulseView";
import type { RelayAgent, UserProfileSummary } from "@/shared/api/types";
import { Button } from "@/shared/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";

type PulseTabBarProps = {
  activeTab: PulseTab;
  getPanelId: (tab: PulseTab) => string;
  getTabId: (tab: PulseTab) => string;
  agentFilter: string | null;
  profiles: Record<string, UserProfileSummary>;
  relayAgents: RelayAgent[];
  onAgentFilterChange: (pubkey: string | null) => void;
  onTabChange: (tab: PulseTab) => void;
};

const tabButtonClassName =
  "h-7 rounded-full border border-transparent px-1.5 text-[10.5px] font-medium text-muted-foreground data-[active=true]:border-border/70 data-[active=true]:bg-background/80 data-[active=true]:text-foreground data-[active=true]:shadow-xs data-[active=true]:backdrop-blur-sm";

function AgentFilter({
  agents,
  profiles,
  selectedPubkey,
  onSelect,
}: {
  agents: RelayAgent[];
  profiles: Record<string, UserProfileSummary>;
  selectedPubkey: string | null;
  onSelect: (pubkey: string | null) => void;
}) {
  const selectedName = selectedPubkey
    ? (profiles[selectedPubkey.toLowerCase()]?.displayName ??
      agents.find((a) => a.pubkey === selectedPubkey)?.name ??
      `${selectedPubkey.slice(0, 8)}...`)
    : null;

  return (
    <DropdownMenu modal={false}>
      <DropdownMenuTrigger asChild>
        <Button
          className="h-7 gap-1.5 px-2 text-xs"
          size="sm"
          variant={selectedPubkey ? "secondary" : "ghost"}
        >
          <Filter className="h-3 w-3" />
          {selectedName ?? "All agents"}
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="max-h-48 overflow-y-auto">
        <DropdownMenuItem onClick={() => onSelect(null)}>
          {!selectedPubkey ? (
            <Check className="h-3.5 w-3.5" />
          ) : (
            <span className="h-3.5 w-3.5" />
          )}
          All agents
        </DropdownMenuItem>
        {agents.map((agent) => {
          const name =
            profiles[agent.pubkey.toLowerCase()]?.displayName ??
            agent.name ??
            `${agent.pubkey.slice(0, 8)}...`;
          const isSelected = selectedPubkey === agent.pubkey;
          return (
            <DropdownMenuItem
              key={agent.pubkey}
              onClick={() => onSelect(agent.pubkey)}
            >
              {isSelected ? (
                <Check className="h-3.5 w-3.5" />
              ) : (
                <span className="h-3.5 w-3.5" />
              )}
              {name}
            </DropdownMenuItem>
          );
        })}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

export function PulseTabBar({
  activeTab,
  agentFilter,
  getPanelId,
  getTabId,
  profiles,
  relayAgents,
  onAgentFilterChange,
  onTabChange,
}: PulseTabBarProps) {
  return (
    <div className="relative z-40 shrink-0 px-4 pt-2 sm:px-6">
      <div className="relative mx-auto flex w-full max-w-2xl items-center justify-center">
        <div className="min-w-0 max-w-full">
          <div className="-mx-4 overflow-x-auto px-4 [scrollbar-width:none] [&::-webkit-scrollbar]:hidden">
            <div
              aria-label="Pulse sections"
              className="flex items-center gap-1"
              role="tablist"
            >
              <Button
                aria-controls={getPanelId("search")}
                aria-label="Search Pulse"
                aria-selected={activeTab === "search"}
                className="h-7 w-7 shrink-0 rounded-full border border-transparent p-0 text-muted-foreground data-[active=true]:border-border/70 data-[active=true]:bg-background/80 data-[active=true]:text-foreground data-[active=true]:shadow-xs data-[active=true]:backdrop-blur-sm"
                data-active={activeTab === "search"}
                id={getTabId("search")}
                onClick={() => onTabChange("search")}
                role="tab"
                size="sm"
                type="button"
                variant="ghost"
              >
                <Search className="h-4 w-4" />
              </Button>
              <Button
                aria-controls={getPanelId("everyone")}
                aria-selected={activeTab === "everyone"}
                className={tabButtonClassName}
                data-active={activeTab === "everyone"}
                id={getTabId("everyone")}
                onClick={() => onTabChange("everyone")}
                role="tab"
                size="sm"
                type="button"
                variant="ghost"
              >
                Everyone
              </Button>
              <Button
                aria-controls={getPanelId("people")}
                aria-selected={activeTab === "people"}
                className={tabButtonClassName}
                data-active={activeTab === "people"}
                id={getTabId("people")}
                onClick={() => onTabChange("people")}
                role="tab"
                size="sm"
                type="button"
                variant="ghost"
              >
                Following
              </Button>
              <Button
                aria-controls={getPanelId("liked")}
                aria-selected={activeTab === "liked"}
                className={tabButtonClassName}
                data-active={activeTab === "liked"}
                id={getTabId("liked")}
                onClick={() => onTabChange("liked")}
                role="tab"
                size="sm"
                type="button"
                variant="ghost"
              >
                Liked
              </Button>
              <Button
                aria-controls={getPanelId("agents")}
                aria-selected={activeTab === "agents"}
                className={tabButtonClassName}
                data-active={activeTab === "agents"}
                id={getTabId("agents")}
                onClick={() => onTabChange("agents")}
                role="tab"
                size="sm"
                type="button"
                variant="ghost"
              >
                Agents
                {relayAgents.length > 0 ? (
                  <span className="ml-1.5 inline-flex h-4 min-w-4 items-center justify-center rounded-full bg-muted px-1 text-[10px] font-medium text-muted-foreground">
                    {relayAgents.length}
                  </span>
                ) : null}
              </Button>
              <Button
                aria-controls={getPanelId("mine")}
                aria-selected={activeTab === "mine"}
                className={tabButtonClassName}
                data-active={activeTab === "mine"}
                id={getTabId("mine")}
                onClick={() => onTabChange("mine")}
                role="tab"
                size="sm"
                type="button"
                variant="ghost"
              >
                Mine
              </Button>
            </div>
          </div>
        </div>

        <div className="absolute right-0 flex items-center gap-1">
          {activeTab === "agents" && relayAgents.length > 1 ? (
            <AgentFilter
              agents={relayAgents}
              onSelect={onAgentFilterChange}
              profiles={profiles}
              selectedPubkey={agentFilter}
            />
          ) : null}
        </div>
      </div>
    </div>
  );
}
