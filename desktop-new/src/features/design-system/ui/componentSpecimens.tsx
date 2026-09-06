import {
  IconDots,
  IconHash,
  IconMessageCircle,
  IconPlus,
  IconSettings,
} from "@tabler/icons-react";
import type { ReactNode } from "react";
import { useState } from "react";
import { Avatar } from "@/shared/ui/Avatar";
import { InlineTile } from "@/shared/ui/InlineTile";
import { Button } from "@/shared/ui/Button";
import { IconButton } from "@/shared/ui/IconButton";
import { NavigatorRow } from "@/shared/ui/NavigatorRow";
import { NavigatorSection } from "@/shared/ui/NavigatorSection";
import { PanelHeader } from "@/shared/ui/PanelHeader";
import { SearchField } from "@/shared/ui/SearchField";
import { SegmentedNavigation } from "@/shared/ui/SegmentedNavigation";
import { WorkspaceSurface } from "@/shared/ui/WorkspaceSurface";
import type { TileAddress } from "@/shared/tiles/address";
import { tileFaces } from "@/shared/tiles/faceResolver";

const DESTINATIONS = [
  { value: "home", label: "Home" },
  { value: "messages", label: "Messages" },
  { value: "projects", label: "Projects" },
] as const;

type Destination = (typeof DESTINATIONS)[number]["value"];

function SpecimenFrame({
  children,
  backdrop = false,
}: {
  children: ReactNode;
  backdrop?: boolean;
}) {
  return (
    <div
      className="component-specimen-frame"
      data-backdrop={backdrop || undefined}
    >
      {children}
    </div>
  );
}

function SpecimenGroup({
  label,
  children,
}: {
  label: string;
  children: ReactNode;
}) {
  return (
    <section className="component-specimen-group">
      <h2 className="text-body-sm text-tertiary">{label}</h2>
      <SpecimenFrame>{children}</SpecimenFrame>
    </section>
  );
}

function ButtonSpecimen() {
  return (
    <div className="component-specimen-stack">
      <SpecimenGroup label="Variants">
        <div className="component-specimen-row">
          <Button variant="primary">Primary</Button>
          <Button variant="quiet">Quiet</Button>
          <Button variant="ghost">Ghost</Button>
        </div>
      </SpecimenGroup>
      <SpecimenGroup label="Sizes">
        <div className="component-specimen-row">
          <Button variant="quiet" size="compact">
            Compact
          </Button>
          <Button variant="quiet">Default</Button>
        </div>
      </SpecimenGroup>
      <SpecimenGroup label="States">
        <div className="component-specimen-row">
          <Button variant="primary">Enabled</Button>
          <Button variant="quiet" disabled>
            Disabled
          </Button>
        </div>
      </SpecimenGroup>
    </div>
  );
}
function IconButtonSpecimen() {
  const icons = {
    add: <IconPlus size={16} stroke={1.7} aria-hidden="true" />,
    more: <IconDots size={16} stroke={1.7} aria-hidden="true" />,
    settings: <IconSettings size={16} stroke={1.7} aria-hidden="true" />,
  };
  return (
    <div className="component-specimen-stack">
      <SpecimenGroup label="Variants">
        <div className="component-specimen-row">
          <IconButton aria-label="Quiet add" icon={icons.add} variant="quiet" />
          <IconButton
            aria-label="Ghost more"
            icon={icons.more}
            variant="ghost"
          />
          <IconButton aria-label="Solid add" icon={icons.add} variant="solid" />
          <IconButton
            aria-label="Chrome settings"
            icon={icons.settings}
            variant="chrome"
          />
        </div>
      </SpecimenGroup>
      <SpecimenGroup label="Sizes">
        <div className="component-specimen-row">
          <IconButton
            aria-label="Compact settings"
            icon={icons.settings}
            size="compact"
          />
          <IconButton aria-label="Default settings" icon={icons.settings} />
          <IconButton
            aria-label="Large settings"
            icon={icons.settings}
            size="large"
          />
        </div>
      </SpecimenGroup>
      <SpecimenGroup label="States">
        <div className="component-specimen-row">
          <IconButton
            aria-label="Enabled add"
            icon={icons.add}
            variant="quiet"
          />
          <IconButton
            aria-label="Disabled add"
            icon={icons.add}
            variant="quiet"
            disabled
          />
        </div>
      </SpecimenGroup>
    </div>
  );
}
function AvatarSpecimen() {
  return (
    <div className="component-specimen-stack">
      <SpecimenGroup label="Sizes">
        <div className="component-specimen-row">
          <Avatar
            src="/design-system/morgan.png"
            alt="Morgan Martin"
            fallback="Morgan"
            size="small"
          />
          <Avatar
            src="/design-system/morgan.png"
            alt="Morgan Martin"
            fallback="Morgan"
          />
          <Avatar
            src="/design-system/morgan.png"
            alt="Morgan Martin"
            fallback="Morgan"
            size="large"
          />
        </div>
      </SpecimenGroup>
      <SpecimenGroup label="Fallback">
        <div className="component-specimen-row">
          <Avatar alt="Cynthia Chen" fallback="Cynthia" size="small" />
          <Avatar alt="Cynthia Chen" fallback="Cynthia" />
          <Avatar alt="Cynthia Chen" fallback="Cynthia" size="large" />
        </div>
      </SpecimenGroup>
    </div>
  );
}
function WorkspaceSurfaceSpecimen() {
  return (
    <div className="component-specimen-stack">
      <SpecimenGroup label="Panel">
        <div className="component-single-surface-demo">
          <WorkspaceSurface aria-label="Standalone panel example">
            <PanelHeader variant="compact" title="Standalone panel" />
          </WorkspaceSurface>
        </div>
      </SpecimenGroup>
      <SpecimenGroup label="Connected edges">
        <div className="component-surface-demo">
          <WorkspaceSurface
            as="aside"
            variant="connected-right"
            aria-label="Right-connected surface example"
          >
            <PanelHeader variant="compact" title="Navigator" />
          </WorkspaceSurface>
          <WorkspaceSurface
            variant="connected-left"
            aria-label="Left-connected surface example"
          >
            <PanelHeader variant="compact" title="Conversation" />
          </WorkspaceSurface>
        </div>
      </SpecimenGroup>
    </div>
  );
}

function SegmentedNavigationSpecimen() {
  const [destination, setDestination] = useState<Destination>("messages");
  const [iconDestination, setIconDestination] = useState<Destination>("home");
  const iconItems = DESTINATIONS.map((item) => ({
    ...item,
    icon: <IconMessageCircle size={16} stroke={1.7} aria-hidden="true" />,
  }));
  return (
    <div className="component-specimen-stack">
      <section className="component-specimen-group">
        <h2 className="text-body-sm text-tertiary">Labels</h2>
        <SpecimenFrame backdrop>
          <SegmentedNavigation
            value={destination}
            items={DESTINATIONS}
            label="Prototype destinations"
            onValueChange={setDestination}
          />
        </SpecimenFrame>
      </section>
      <section className="component-specimen-group">
        <h2 className="text-body-sm text-tertiary">
          Icons and trailing action
        </h2>
        <SpecimenFrame backdrop>
          <SegmentedNavigation
            value={iconDestination}
            items={iconItems}
            label="Prototype destinations with icons"
            onValueChange={setIconDestination}
            trailingAction={
              <IconButton
                aria-label="Create"
                icon={<IconPlus size={16} stroke={1.7} aria-hidden="true" />}
                size="compact"
              />
            }
          />
        </SpecimenFrame>
      </section>
    </div>
  );
}

function PanelHeaderSpecimen() {
  const actions = (
    <IconButton
      aria-label="More conversation actions"
      icon={<IconDots size={16} stroke={1.7} aria-hidden="true" />}
      size="compact"
    />
  );
  return (
    <div className="component-specimen-stack">
      <SpecimenGroup label="Default">
        <WorkspaceSurface aria-label="Default panel header example">
          <PanelHeader
            title="Conversation"
            icon={
              <IconMessageCircle size={16} stroke={1.7} aria-hidden="true" />
            }
            actions={actions}
          />
        </WorkspaceSurface>
      </SpecimenGroup>
      <SpecimenGroup label="Compact">
        <WorkspaceSurface aria-label="Compact panel header example">
          <PanelHeader variant="compact" title="Thread" actions={actions} />
        </WorkspaceSurface>
      </SpecimenGroup>
    </div>
  );
}

function SearchFieldSpecimen() {
  const [emptyQuery, setEmptyQuery] = useState("");
  const [filledQuery, setFilledQuery] = useState("design");
  return (
    <div className="component-specimen-stack">
      <SpecimenGroup label="Empty">
        <div className="component-field-demo">
          <SearchField
            value={emptyQuery}
            onValueChange={setEmptyQuery}
            label="Find a channel"
            placeholder="Search"
          />
        </div>
      </SpecimenGroup>
      <SpecimenGroup label="Filled and clearable">
        <div className="component-field-demo">
          <SearchField
            value={filledQuery}
            onValueChange={setFilledQuery}
            label="Find a channel"
            placeholder="Search"
          />
        </div>
      </SpecimenGroup>
    </div>
  );
}

function NavigatorSectionSpecimen() {
  return (
    <div className="component-specimen-stack">
      <SpecimenGroup label="With rows">
        <div className="component-navigator-section-demo">
          <NavigatorSection label="Pinned">
            <NavigatorRow
              label="buzz-design"
              icon={<IconHash size={16} stroke={1.7} aria-hidden="true" />}
            />
            <NavigatorRow
              label="desktop-new"
              icon={<IconHash size={16} stroke={1.7} aria-hidden="true" />}
            />
          </NavigatorSection>
        </div>
      </SpecimenGroup>
      <SpecimenGroup label="Adjacent sections">
        <div className="component-navigator-section-demo">
          <NavigatorSection label="Projects">
            <NavigatorRow label="berd-main" />
          </NavigatorSection>
          <NavigatorSection label="Personal">
            <NavigatorRow label="design-system" />
          </NavigatorSection>
        </div>
      </SpecimenGroup>
    </div>
  );
}

function NavigatorRowSpecimen() {
  const [selected, setSelected] = useState("buzz-design");
  return (
    <div className="component-specimen-stack">
      <SpecimenGroup label="Default, selected, and metadata">
        <div className="component-navigator-section-demo">
          <NavigatorRow
            label="buzz-design"
            icon={<IconHash size={16} stroke={1.7} aria-hidden="true" />}
            selected={selected === "buzz-design"}
            onClick={() => setSelected("buzz-design")}
          />
          <NavigatorRow
            label="desktop-new"
            icon={<IconHash size={16} stroke={1.7} aria-hidden="true" />}
            trailing={<span className="text-body-sm">3</span>}
            selected={selected === "desktop-new"}
            onClick={() => setSelected("desktop-new")}
          />
        </div>
      </SpecimenGroup>
      <SpecimenGroup label="Inset">
        <div className="component-navigator-section-demo">
          <NavigatorRow
            label="Session interaction model"
            icon={
              <IconMessageCircle size={16} stroke={1.7} aria-hidden="true" />
            }
            inset
            selected={selected === "session"}
            onClick={() => setSelected("session")}
          />
        </div>
      </SpecimenGroup>
    </div>
  );
}

const TILE_PERSON: TileAddress = { kind: "person", id: "pk-morgan" };
const TILE_AGENT: TileAddress = { kind: "agent", id: "pk-vogue" };
const TILE_CHANNEL: TileAddress = { kind: "channel", id: "ch-design" };
const TILE_UNRESOLVED: TileAddress = {
  kind: "person",
  id: "9f2c4a1b7e5d8306a4b2c1d9e8f7a6b5c4d3e2f1a0b9c8d7e6f5a4b3c2d1e0f9",
};
const TILE_LONG: TileAddress = { kind: "person", id: "pk-long" };

/**
 * Seeds faces so the specimens show resolved tiles. The product resolves these
 * from real identity; the page only needs the shapes to be visible.
 */
function seedTileFaces() {
  tileFaces.put(TILE_PERSON, {
    label: "Morgan",
    status: "online",
    loading: false,
    resolved: true,
  });
  tileFaces.put(TILE_AGENT, {
    label: "Vogue",
    status: "busy",
    loading: false,
    resolved: true,
  });
  tileFaces.put(TILE_CHANNEL, {
    label: "design",
    loading: false,
    resolved: true,
  });
  tileFaces.put(TILE_LONG, {
    label: "A deliberately very long display name that must truncate",
    loading: false,
    resolved: true,
  });
}

function InlineTileSpecimen() {
  const [activated, setActivated] = useState<string | null>(null);
  seedTileFaces();

  return (
    <div className="component-specimen-stack">
      <SpecimenGroup label="Kinds">
        <div className="component-specimen-row">
          <InlineTile address={TILE_PERSON} />
          <InlineTile address={TILE_AGENT} />
          <InlineTile address={TILE_CHANNEL} />
        </div>
      </SpecimenGroup>

      <SpecimenGroup label="In a sentence">
        <p className="text-body text-primary">
          Asked <InlineTile address={TILE_PERSON} /> and{" "}
          <InlineTile address={TILE_AGENT} /> to look at the thread in{" "}
          <InlineTile address={TILE_CHANNEL} /> before the review.
        </p>
      </SpecimenGroup>

      <SpecimenGroup label="Read-only, as a conversation renders it">
        <p className="text-body text-primary">
          A tile in a sent message is not a control when the surface has no
          detail to open:{" "}
          <InlineTile address={TILE_PERSON} interactive={false} />
        </p>
      </SpecimenGroup>

      <SpecimenGroup label="Unresolved identity">
        <div className="component-specimen-row">
          <InlineTile address={TILE_UNRESOLVED} />
        </div>
      </SpecimenGroup>

      <SpecimenGroup label="Long label truncates">
        <div className="component-specimen-row">
          <InlineTile address={TILE_LONG} />
        </div>
      </SpecimenGroup>

      <SpecimenGroup label="Activation">
        <div className="component-specimen-row">
          <InlineTile
            address={TILE_PERSON}
            onActivate={(address) => setActivated(address.id)}
          />
          <span className="text-body-sm text-tertiary">
            {activated ? `Opened ${activated}` : "Not activated"}
          </span>
        </div>
      </SpecimenGroup>
    </div>
  );
}

export const COMPONENT_SPECIMENS: Record<string, () => ReactNode> = {
  button: ButtonSpecimen,
  "icon-button": IconButtonSpecimen,
  avatar: AvatarSpecimen,
  "inline-tile": InlineTileSpecimen,
  "workspace-surface": WorkspaceSurfaceSpecimen,
  "segmented-navigation": SegmentedNavigationSpecimen,
  "panel-header": PanelHeaderSpecimen,
  "search-field": SearchFieldSpecimen,
  "navigator-section": NavigatorSectionSpecimen,
  "navigator-row": NavigatorRowSpecimen,
};
