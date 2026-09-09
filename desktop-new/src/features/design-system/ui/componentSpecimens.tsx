import { FlexWorkspace } from "@/shared/ui/FlexWorkspace";
import {
  ConversationSpecimen,
  AgentWorkSpecimen,
  WorkStepSpecimen,
  ConversationMessageSpecimen,
  ConversationStateSpecimen,
} from "./ConversationSpecimen";
import { BentoSpecimen } from "./BentoSpecimen";
import {
  IconDots,
  IconHash,
  IconMessageCircle,
  IconPlus,
  IconUsers,
  IconSettings,
} from "@tabler/icons-react";
import type { ReactNode } from "react";
import { useState } from "react";
import { Switch } from "@/shared/ui/Switch";
import { Accordion } from "@/shared/ui/Accordion";
import { Select } from "@/shared/ui/Select";
import { OnboardingShowcase } from "@/features/onboarding/ui/OnboardingShowcase";
import { Avatar } from "@/shared/ui/Avatar";
import { InlineChip } from "@/shared/ui/InlineChip";
import { Button } from "@/shared/ui/Button";
import { ConversationHeader } from "@/shared/ui/ConversationHeader";
import { IconButton } from "@/shared/ui/IconButton";
import { NavigationItem } from "@/shared/ui/NavigationItem";
import { NavigationSection } from "@/shared/ui/NavigationSection";
import { PanelHeader } from "@/shared/ui/PanelHeader";
import { SearchField } from "@/shared/ui/SearchField";
import { Tabs } from "@/shared/ui/Tabs";
import { TextField } from "@/shared/ui/TextField";

import { ComponentAnatomy } from "./ComponentAnatomy";
import {
  ActivityRailSpecimen,
  FormattingBarSpecimen,
  MessageComposerSpecimen,
} from "./composerSpecimens";
import { Panel } from "@/shared/ui/Panel";
import { PreviewCard } from "@/shared/ui/PreviewCard";
import type { ChipAddress } from "@/shared/chips/address";
import { chipFaces } from "@/shared/chips/faceResolver";

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

/**
 * One sample with the prop that produced it printed underneath.
 *
 * A row of four icon buttons is unreadable without this — they differ only by
 * fill, so there is no way to tell which one is `solid` and which is `chrome`,
 * and the page is documentation. The caption names the prop as you would type
 * it (`variant="solid"`), not a prose paraphrase, so reading the page tells you
 * what to write.
 */
function Specimen({
  prop,
  children,
}: {
  /** As typed in JSX, e.g. `variant="quiet"`. Omit for a default. */
  prop?: string;
  children: ReactNode;
}) {
  return (
    <div className="component-specimen">
      {children}
      <code className="component-specimen-prop text-mono-sm text-tertiary">
        {prop ?? "default"}
      </code>
    </div>
  );
}

function ButtonSpecimen() {
  return (
    <div className="component-specimen-stack">
      <SpecimenGroup label="Variants">
        <div className="component-specimen-row">
          <Specimen prop='variant="primary"'>
            <Button variant="primary">Save</Button>
          </Specimen>
          <Specimen prop='variant="quiet"'>
            <Button variant="quiet">Save</Button>
          </Specimen>
          <Specimen prop='variant="ghost"'>
            <Button variant="ghost">Save</Button>
          </Specimen>
        </div>
      </SpecimenGroup>
      <SpecimenGroup label="Sizes">
        <div className="component-specimen-row">
          <Specimen prop='size="compact"'>
            <Button variant="quiet" size="compact">
              Save
            </Button>
          </Specimen>
          <Specimen prop='size="default"'>
            <Button variant="quiet">Save</Button>
          </Specimen>
        </div>
      </SpecimenGroup>
      <SpecimenGroup label="States">
        <div className="component-specimen-row">
          <Specimen>
            <Button variant="primary">Save</Button>
          </Specimen>
          <Specimen prop="disabled">
            <Button variant="quiet" disabled>
              Save
            </Button>
          </Specimen>
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
      {/* `chrome` sits on the app backdrop, because it is a translucent glass
          fill — on a flat panel it has nothing to be translucent over and reads
          as a plain grey. */}
      <SpecimenGroup label="Variants">
        <div className="component-specimen-row">
          <Specimen prop='variant="quiet"'>
            <IconButton
              aria-label="Quiet add"
              icon={icons.add}
              variant="quiet"
            />
          </Specimen>
          <Specimen prop='variant="ghost"'>
            <IconButton
              aria-label="Ghost more"
              icon={icons.more}
              variant="ghost"
            />
          </Specimen>
          <Specimen prop='variant="solid"'>
            <IconButton
              aria-label="Solid add"
              icon={icons.add}
              variant="solid"
            />
          </Specimen>
          <Specimen prop='variant="tint" shape="round"'>
            <IconButton
              aria-label="Round tinted send"
              icon={icons.add}
              shape="round"
              variant="tint"
            />
          </Specimen>
        </div>
      </SpecimenGroup>
      <section className="component-specimen-group">
        <h2 className="text-body-sm text-tertiary">
          Chrome, over the workspace backdrop
        </h2>
        <SpecimenFrame backdrop>
          <div className="component-specimen-row">
            <Specimen prop='variant="chrome"'>
              <IconButton
                aria-label="Chrome settings"
                icon={icons.settings}
                variant="chrome"
              />
            </Specimen>
          </div>
        </SpecimenFrame>
      </section>
      {/* Shown on `quiet`, not the default `ghost`: the three sizes differ only
          in hit area (30 / 36 / 40px), and with no fill they render as three
          identical 16px glyphs — the page would claim to show a size ramp while
          showing nothing. A fill makes the box the sample. */}
      <SpecimenGroup label="Sizes">
        <div className="component-specimen-row">
          <Specimen prop='size="compact"'>
            <IconButton
              aria-label="Compact settings"
              icon={icons.settings}
              variant="quiet"
              size="compact"
            />
          </Specimen>
          <Specimen prop='size="toolbar"'>
            <IconButton
              aria-label="Toolbar settings"
              icon={icons.settings}
              variant="quiet"
              size="toolbar"
            />
          </Specimen>
          <Specimen prop='size="default"'>
            <IconButton
              aria-label="Default settings"
              icon={icons.settings}
              variant="quiet"
            />
          </Specimen>
          <Specimen prop='size="large"'>
            <IconButton
              aria-label="Large settings"
              icon={icons.settings}
              variant="quiet"
              size="large"
            />
          </Specimen>
        </div>
      </SpecimenGroup>
      <SpecimenGroup label="States">
        <div className="component-specimen-row">
          <Specimen>
            <IconButton
              aria-label="Enabled add"
              icon={icons.add}
              variant="quiet"
            />
          </Specimen>
          <Specimen prop="disabled">
            <IconButton
              aria-label="Disabled add"
              icon={icons.add}
              variant="quiet"
              disabled
            />
          </Specimen>
          {(["tint", "solid"] as const).map((variant) => (
            <Specimen
              key={variant}
              prop={`variant="${variant}" shape="round" disabled`}
            >
              <IconButton
                aria-label={`Disabled round ${variant}`}
                icon={icons.add}
                shape="round"
                variant={variant}
                disabled
              />
            </Specimen>
          ))}
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
          <Specimen prop='size="small"'>
            <Avatar
              src="/design-system/morgan.png"
              alt="Morgan Martin"
              fallback="Morgan"
              size="small"
            />
          </Specimen>
          <Specimen prop='size="default"'>
            <Avatar
              src="/design-system/morgan.png"
              alt="Morgan Martin"
              fallback="Morgan"
            />
          </Specimen>
          <Specimen prop='size="large"'>
            <Avatar
              src="/design-system/morgan.png"
              alt="Morgan Martin"
              fallback="Morgan"
              size="large"
            />
          </Specimen>
        </div>
      </SpecimenGroup>
      {/* No `src`, so the fallback initial shows. Same three sizes, because a
          fallback has to hold the ramp as well as an image does. */}
      <SpecimenGroup label="Fallback, with no src">
        <div className="component-specimen-row">
          <Specimen prop='size="small"'>
            <Avatar alt="Cynthia Chen" fallback="Cynthia" size="small" />
          </Specimen>
          <Specimen prop='size="default"'>
            <Avatar alt="Cynthia Chen" fallback="Cynthia" />
          </Specimen>
          <Specimen prop='size="large"'>
            <Avatar alt="Cynthia Chen" fallback="Cynthia" size="large" />
          </Specimen>
        </div>
      </SpecimenGroup>
    </div>
  );
}
function PanelSpecimen() {
  return (
    <div className="component-specimen-stack">
      <SpecimenGroup label="Panel — the surface by itself">
        {/* No invented header or copy inside this specimen. Panel owns a surface,
            not what a product chooses to put on it, so an empty Panel is the
            honest live component. The gradient is the surrounding context it is
            designed to sit on, not content supplied by the specimen. */}
        <div className="component-single-surface-demo">
          <Panel aria-label="Panel surface" />
        </div>
      </SpecimenGroup>
    </div>
  );
}

function TextFieldSpecimen() {
  return (
    <div className="component-specimen-stack">
      <SpecimenGroup label="Content">
        <div className="w-full max-w-sm">
          <TextField
            label="Email address"
            placeholder="name@example.com"
            description="We’ll use this to help you sign in."
            type="email"
          />
        </div>
      </SpecimenGroup>
      <SpecimenGroup label="States">
        <div className="component-specimen-stack w-full max-w-sm">
          <Specimen>
            <TextField label="Name" placeholder="Enter your name" />
          </Specimen>
          <Specimen prop='error="Enter a valid email address"'>
            <TextField
              label="Email address"
              defaultValue="not-an-email"
              error="Enter a valid email address"
              type="email"
            />
          </Specimen>
          <Specimen prop="disabled">
            <TextField label="Workspace" defaultValue="Buzz" disabled />
          </Specimen>
        </div>
      </SpecimenGroup>
    </div>
  );
}

/**
 * The parts of Tabs, per variant.
 *
 * Two lists rather than one, because the interesting fact is that the *same*
 * parts are drawn differently — the container and the tab carry no fill at all
 * in `panel`, and the indicator changes from a pill to a bar.
 */
const TABS_CHROME_PARTS = [
  {
    name: "Container",
    selector: ".buzz-tabs",
    show: ["background", "shadow", "blur", "radius"],
    note: "The glass material: fill, blur, and rim arrive together as glass-primary.",
  },
  {
    name: "Selection",
    selector: ".buzz-tabs-indicator",
    show: ["background", "shadow", "radius"],
    note: "An opaque pill behind the selected tab. Opaque because on glass, elevation reads as less translucency.",
  },
  {
    name: "Selected tab",
    selector: ".buzz-tabs-tab[data-selected]",
    show: ["color", "background"],
    note: "Draws no fill of its own — the pill behind it is the selection.",
  },
  {
    name: "Unselected tab",
    selector: ".buzz-tabs-tab:not([data-selected])",
    show: ["color", "background"],
  },
] as const;

const TABS_PANEL_PARTS = [
  {
    name: "Container",
    selector: ".buzz-tabs",
    show: ["background", "shadow", "blur", "radius"],
    note: "Nothing drawn. On a panel the tabs are text and the bar is the only mark.",
  },
  {
    name: "Selection",
    selector: ".buzz-tabs-indicator",
    show: ["background", "radius"],
    note: "A 2px bar on the bottom edge, spanning the selected tab.",
  },
  {
    name: "Selected tab",
    selector: ".buzz-tabs-tab[data-selected]",
    show: ["color", "background"],
    note: "Same size as unselected — selection is colour and the bar, never weight, which would reflow the row.",
  },
  {
    name: "Unselected tab",
    selector: ".buzz-tabs-tab:not([data-selected])",
    show: ["color", "background"],
  },
] as const;

function TabsSpecimen() {
  const [destination, setDestination] = useState<Destination>("messages");
  const [panelDestination, setPanelDestination] =
    useState<Destination>("messages");
  const [iconDestination, setIconDestination] = useState<Destination>("home");
  const iconItems = DESTINATIONS.map((item) => ({
    ...item,
    icon: <IconMessageCircle size={16} stroke={1.7} aria-hidden="true" />,
  }));
  return (
    <div className="component-specimen-stack">
      <section className="component-specimen-group">
        <h2 className="text-body-sm text-tertiary">
          Chrome — a glass pill, for the app backdrop
        </h2>
        <SpecimenFrame backdrop>
          <div data-anatomy="tabs-chrome">
            <Tabs
              value={destination}
              items={DESTINATIONS}
              label="Prototype destinations"
              onValueChange={setDestination}
            />
          </div>
        </SpecimenFrame>
        <ComponentAnatomy
          scope='[data-anatomy="tabs-chrome"]'
          caption="Which token draws each part of the chrome variant, read from the specimen above."
          parts={TABS_CHROME_PARTS}
        />
      </section>
      {/* On a panel rather than the backdrop, because that is the whole point of
          the variant — and because this specimen showing only the chrome version
          on only the gradient is why the panel failure went unnoticed until it
          appeared on a real page. */}
      <section className="component-specimen-group">
        <h2 className="text-body-sm text-tertiary">
          Panel — an underline, for a plain surface
        </h2>
        <SpecimenFrame>
          <div data-anatomy="tabs-panel">
            <Tabs
              value={panelDestination}
              items={DESTINATIONS}
              label="Prototype destinations on a panel"
              onValueChange={setPanelDestination}
              variant="panel"
            />
          </div>
        </SpecimenFrame>
        <ComponentAnatomy
          scope='[data-anatomy="tabs-panel"]'
          caption="The same parts in the panel variant. The container and the tab draw nothing at all, which is what makes this legible on a plain surface: there is no second fill to collide with the first."
          parts={TABS_PANEL_PARTS}
        />
      </section>
      <section className="component-specimen-group">
        <h2 className="text-body-sm text-tertiary">
          Chrome, with icons and a trailing action outside the selection
        </h2>
        <SpecimenFrame backdrop>
          <Tabs
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

function ConversationHeaderSpecimen() {
  const people = (
    <IconButton
      aria-label="View conversation participants"
      icon={<IconUsers size={16} stroke={1.7} aria-hidden="true" />}
      size="compact"
    />
  );
  return (
    <div className="component-specimen-stack">
      <SpecimenGroup label="Channel — identity and people action">
        <ConversationHeader
          icon={<IconHash size={16} stroke={1.7} />}
          title="design"
          actions={people}
        />
      </SpecimenGroup>
      <SpecimenGroup label="Session — title status and contextual action">
        <ConversationHeader
          icon={<IconMessageCircle size={16} stroke={1.7} />}
          title="Review agent setup"
          metadata={<span className="device-label">On this device</span>}
          actions={
            <>
              {people}
              <Button variant="quiet" size="compact">
                Start session
              </Button>
            </>
          }
        />
      </SpecimenGroup>
      <SpecimenGroup label="Origin context — only when the title alone is ambiguous">
        <ConversationHeader
          icon={<IconMessageCircle size={16} stroke={1.7} />}
          title="New session"
          context={
            <>
              <IconHash size={13} stroke={1.6} aria-hidden="true" /> From design
            </>
          }
        />
      </SpecimenGroup>
      <SpecimenGroup label="Narrow — full context and actions remain available">
        <div className="w-full max-w-xs">
          <ConversationHeader
            title="Review the conversation hierarchy"
            metadata="On this device"
            context="From design"
            actions={
              <>
                {people}
                <Button variant="quiet" size="compact">
                  Start session
                </Button>
              </>
            }
          />
        </div>
      </SpecimenGroup>
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
      {/* PanelHeader is the header row itself — optional icon, title, and actions.
          Panel is intentionally not wrapped around it: Panel owns the surface and
          PanelHeader owns neither its container nor the content below it. The end
          control is the existing IconButton component, composed through `actions`
          rather than rebuilt as a raw button. */}
      <SpecimenGroup label="Default — icon, title, and IconButton action">
        <PanelHeader
          title="Conversation"
          icon={<IconMessageCircle size={16} stroke={1.7} aria-hidden="true" />}
          actions={actions}
        />
      </SpecimenGroup>
      <SpecimenGroup label="Compact — title and IconButton action">
        <PanelHeader variant="compact" title="Thread" actions={actions} />
      </SpecimenGroup>
    </div>
  );
}

function SearchFieldSpecimen() {
  const [emptyQuery, setEmptyQuery] = useState("");
  const [filledQuery, setFilledQuery] = useState("design");
  return (
    <div className="component-specimen-stack">
      <SpecimenGroup label='value="" — the clear action is absent'>
        <div className="component-field-demo">
          <SearchField
            value={emptyQuery}
            onValueChange={setEmptyQuery}
            label="Find a channel"
            placeholder="Search"
          />
        </div>
      </SpecimenGroup>
      <SpecimenGroup label='value="design" — the clear action appears'>
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

function NavigationSectionSpecimen() {
  return (
    <div className="component-specimen-stack">
      <SpecimenGroup label='label="Pinned", with rows as children'>
        <div className="component-navigation-section-demo">
          <NavigationSection label="Pinned">
            <NavigationItem
              label="buzz-design"
              icon={<IconHash size={16} stroke={1.7} aria-hidden="true" />}
            />
            <NavigationItem
              label="desktop-new"
              icon={<IconHash size={16} stroke={1.7} aria-hidden="true" />}
            />
          </NavigationSection>
        </div>
      </SpecimenGroup>
      <SpecimenGroup label="Two sections adjacent — the gap between them is the grouping">
        <div className="component-navigation-section-demo">
          <NavigationSection label="Projects">
            <NavigationItem label="berd-main" />
          </NavigationSection>
          <NavigationSection label="Personal">
            <NavigationItem label="design-system" />
          </NavigationSection>
        </div>
      </SpecimenGroup>
    </div>
  );
}

function NavigationItemSpecimen() {
  const [selected, setSelected] = useState("buzz-design");
  return (
    <div className="component-specimen-stack">
      {/* Interactive: clicking moves `selected`, so the selected fill and the
          unselected rest state are both visible at once and comparable. */}
      <SpecimenGroup label="Selected, and trailing for metadata — click to move the selection">
        <div className="component-navigation-section-demo">
          <NavigationItem
            label="buzz-design"
            icon={<IconHash size={16} stroke={1.7} aria-hidden="true" />}
            selected={selected === "buzz-design"}
            onClick={() => setSelected("buzz-design")}
          />
          <NavigationItem
            label="desktop-new"
            icon={<IconHash size={16} stroke={1.7} aria-hidden="true" />}
            trailing={<span className="text-body-sm">3</span>}
            selected={selected === "desktop-new"}
            onClick={() => setSelected("desktop-new")}
          />
        </div>
      </SpecimenGroup>
      <SpecimenGroup label="Inset — one level of nesting under a row">
        <div className="component-navigation-section-demo">
          <NavigationItem
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

const CHIP_PERSON: ChipAddress = { kind: "person", id: "pk-morgan" };
const CHIP_AGENT: ChipAddress = { kind: "agent", id: "pk-morgarita" };
const CHIP_CHANNEL: ChipAddress = { kind: "channel", id: "ch-buzz-team" };
const CHIP_MESSAGE: ChipAddress = { kind: "message", id: "ev-thread-root" };
const CHIP_LINK: ChipAddress = {
  kind: "link",
  id: "https://github.com/block/buzz",
};
const CHIP_UNRESOLVED: ChipAddress = {
  kind: "person",
  id: "9f2c4a1b7e5d8306a4b2c1d9e8f7a6b5c4d3e2f1a0b9c8d7e6f5a4b3c2d1e0f9",
};
const CHIP_LONG: ChipAddress = { kind: "person", id: "pk-long" };

const CHIP_KIND_ROWS = [
  { label: "Person", address: CHIP_PERSON },
  { label: "Agent", address: CHIP_AGENT },
  { label: "Channel", address: CHIP_CHANNEL },
  { label: "Message", address: CHIP_MESSAGE },
  { label: "Link", address: CHIP_LINK },
] as const;

/**
 * Seeds faces so the specimens show resolved chips. The product resolves these
 * from real identity; the page only needs the shapes to be visible.
 */
function seedChipFaces() {
  chipFaces.put(CHIP_PERSON, {
    label: "Morgan Martin",
    loading: false,
    resolved: true,
  });
  chipFaces.put(CHIP_AGENT, {
    label: "Morgarita",
    loading: false,
    resolved: true,
  });
  chipFaces.put(CHIP_CHANNEL, {
    label: "buzz-team",
    loading: false,
    resolved: true,
  });
  chipFaces.put(CHIP_MESSAGE, {
    label: "buzz-team",
    loading: false,
    resolved: true,
  });
  chipFaces.put(CHIP_LINK, {
    label: "github.com/block/buzz",
    loading: false,
    resolved: true,
  });
  chipFaces.put(CHIP_LONG, {
    label: "A deliberately very long display name that must truncate",
    loading: false,
    resolved: true,
  });
}

function PreviewCardSpecimen() {
  return (
    <div className="component-specimen-stack">
      <SpecimenGroup label="Default">
        <p className="text-body text-primary">
          <PreviewCard
            trigger={
              <button type="button" className="buzz-preview-card-example">
                Morgan Martin
              </button>
            }
          >
            <span className="buzz-preview-card-kind text-body-sm text-tertiary">
              Person
            </span>
            <span className="buzz-preview-card-name text-body text-primary">
              Morgan Martin
            </span>
            <span className="text-body-sm text-secondary">
              Product designer
            </span>
          </PreviewCard>{" "}
          is reviewing the first Composer composition.
        </p>
      </SpecimenGroup>
    </div>
  );
}

function InlineChipSpecimen() {
  const [activated, setActivated] = useState<string | null>(null);
  seedChipFaces();

  return (
    <div className="component-specimen-stack">
      <SpecimenGroup label="Kinds">
        {/* Caption under the sample, not in a fixed-width column beside it: a
            label column wide enough for `kind: "channel"` left the link chip
            nowhere to truncate and pushed it off a 380px viewport. */}
        <div className="component-specimen-row">
          {CHIP_KIND_ROWS.map((row) => (
            <Specimen key={row.label} prop={`kind: "${row.address.kind}"`}>
              <InlineChip address={row.address} />
            </Specimen>
          ))}
        </div>
      </SpecimenGroup>

      <SpecimenGroup label="In a sentence">
        <p className="text-body text-primary">
          Asked <InlineChip address={CHIP_PERSON} /> and{" "}
          <InlineChip address={CHIP_AGENT} /> to look at{" "}
          <InlineChip address={CHIP_MESSAGE} /> in{" "}
          <InlineChip address={CHIP_CHANNEL} />, alongside{" "}
          <InlineChip address={CHIP_LINK} /> before the review.
        </p>
      </SpecimenGroup>

      <SpecimenGroup label="Resolved preview">
        <p className="text-body text-primary">
          Hover or focus <InlineChip address={CHIP_PERSON} /> to reveal its
          quiet preview. It supplies context without opening another surface.
        </p>
      </SpecimenGroup>

      <SpecimenGroup label="Inert rendering">
        <p className="text-body text-primary">
          A surface that cannot honestly reveal reference detail renders an
          inert chip: <InlineChip address={CHIP_PERSON} interactive={false} />
        </p>
      </SpecimenGroup>

      <SpecimenGroup label="Unresolved reference">
        <div className="flex flex-col gap-3">
          <div className="component-specimen-row">
            <InlineChip address={CHIP_UNRESOLVED} />
          </div>
          <p className="max-w-md text-body-sm text-tertiary">
            No name to show and nothing to open, so it reads as inert and
            announces itself as unresolved rather than speaking an identity
            fragment aloud.
          </p>
        </div>
      </SpecimenGroup>

      <SpecimenGroup label="Long label truncates">
        <div className="component-specimen-row">
          <InlineChip address={CHIP_LONG} />
        </div>
      </SpecimenGroup>

      <SpecimenGroup label="Wrapping across lines">
        <p className="max-w-xs text-body text-primary">
          A chip sits in the text flow, so a line break falls before or after it
          and never inside it: <InlineChip address={CHIP_PERSON} />{" "}
          <InlineChip address={CHIP_AGENT} /> <InlineChip address={CHIP_LINK} />
        </p>
      </SpecimenGroup>

      <SpecimenGroup label="Activation">
        <div className="component-specimen-row">
          <InlineChip
            address={CHIP_PERSON}
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

function SwitchSpecimen() {
  const [checked, setChecked] = useState(false);
  return (
    <div className="component-specimen-stack">
      <Switch
        checked={checked}
        onCheckedChange={setChecked}
        label="Show agent activity"
      />
      <Switch checked label="Show agent activity" />
      <Switch disabled label="Show agent activity" />
    </div>
  );
}

function SelectSpecimen() {
  const [value, setValue] = useState("channel");
  return (
    <Select
      label="Preview context"
      value={value}
      onValueChange={setValue}
      groups={[
        {
          label: "Destination",
          options: [
            { value: "channel", label: "Channel" },
            { value: "session", label: "Session" },
            { value: "dm", label: "Direct message" },
          ],
        },
      ]}
    />
  );
}

export const COMPONENT_SPECIMENS: Record<string, () => ReactNode> = {
  "flex-workspace": FlexWorkspace,
  conversation: ConversationSpecimen,
  "conversation-message": ConversationMessageSpecimen,
  "conversation-state": ConversationStateSpecimen,
  "agent-work-area": AgentWorkSpecimen,
  "work-step": WorkStepSpecimen,
  workspace: BentoSpecimen,
  button: ButtonSpecimen,
  "icon-button": IconButtonSpecimen,
  avatar: AvatarSpecimen,
  "preview-card": PreviewCardSpecimen,
  "inline-chip": InlineChipSpecimen,
  panel: PanelSpecimen,
  tabs: TabsSpecimen,
  select: SelectSpecimen,
  switch: SwitchSpecimen,
  accordion: () => (
    <Accordion
      items={[
        {
          value: "purpose",
          title: "When to use an accordion",
          content: (
            <p className="text-body">
              Use a disclosure for supporting content that does not need to be
              visible all the time.
            </p>
          ),
        },
        {
          value: "behavior",
          title: "Keyboard behavior",
          content: (
            <p className="text-body">
              Focus a heading and press Enter or Space to expand it.
            </p>
          ),
        },
      ]}
    />
  ),
  "conversation-header": ConversationHeaderSpecimen,
  "panel-header": PanelHeaderSpecimen,
  "text-field": TextFieldSpecimen,
  "search-field": SearchFieldSpecimen,
  "navigation-section": NavigationSectionSpecimen,
  "navigation-item": NavigationItemSpecimen,
  composer: MessageComposerSpecimen,
  "composer-formatting-bar": FormattingBarSpecimen,
  "activity-rail": ActivityRailSpecimen,
  onboarding: OnboardingShowcase,
};
