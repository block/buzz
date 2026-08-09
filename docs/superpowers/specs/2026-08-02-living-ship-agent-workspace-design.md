# Living Ship Agent Workspace

**Status:** Approved design

**Date:** 2026-08-02

**Product:** Command Adviser for HMAS Supply

## Purpose

Add a lightweight, truthful visual workspace where the user can see which
Command Team advisers are idle, working independently, collaborating, moving
between workspaces, waking, or unavailable.

The primary experience is a pixel-art side elevation of HMAS Supply based on
the supplied HMAS Stalwart incident-board drawing. The complete ship silhouette
is a defining visual feature. Enlarged compartments are embedded inside that
outline at approximate fore-and-aft and vertical positions; exact compartment
geometry is not required.

This is an operational status view, not a simulation. It must not invent work,
collaboration, availability, or agent location when telemetry is missing.

## Product Principles

1. **Truth before ambience.** Working and collaboration claims require
   supporting runtime or observer state.
2. **Recognisable ship first.** The final art preserves the continuous hull,
   flight deck, replenishment rigs, and forward and aft superstructures.
3. **Readable rooms over exact plans.** Rooms are enlarged and regularised so
   agents and activity remain legible at desktop-window sizes.
4. **Reuse Buzz state.** Existing observer ingestion, active-turn tracking,
   runtime status, profiles, navigation, and activity panels remain the sources
   of truth.
5. **A viable visual product.** No game engine, physics, procedural simulation,
   continuous inference, or unrelated assurance work belongs in the MVP.

## Screen and Art Direction

Command Adviser adds a **Ship** sidebar destination. It opens one responsive
side-elevation cutaway containing the entire ship.

The final art uses a fixed logical canvas, initially 1920 by 720, and scales it
responsively with crisp pixel rendering. The artwork is layered:

- one continuous ship exterior and cutaway shell;
- room-interior layers and clickable room hotspots;
- predefined passage and ladder routes;
- eight adviser sprite sets;
- status, selection, and accessibility overlays rendered by React.

The technical drawing is a spatial reference only. The shipped asset is
original cohesive pixel art rather than a technical drawing with boxes placed
over it.

### Compartment Blocking

The cutaway has two deliberately regular room modules.

**Aft module: one column by two rows**

1. DSE Operator Room
2. Plans Room, visually based on the Level 01 Officers Study Room

**Forward module: two columns by three rows**

| Row | Port/left room | Starboard/right room |
|---|---|---|
| Top | C.I.C. | Chart House (drawing label: Chart Room) |
| Middle | Wardroom | Meeting Room |
| Bottom | Ship's Office | Supply Office |

The room modules sit inside the recognisable side silhouette and are connected
by simple visible passages and ladders. Their proportions are selected for
readability, not plan fidelity.

## Adviser Roster and Home Workspaces

The screen includes all eight current Command Team advisers.

| Adviser | Home workspace |
|---|---|
| Chief of Staff | Meeting Room |
| Operations Adviser | C.I.C. |
| Maritime N2 Adviser | DSE Operator Room |
| Navigation Adviser | Chart House |
| Logistics Adviser | Supply Office |
| Plans Adviser | Plans Room |
| Daily Routine Adviser | Ship's Office |
| Reporting Adviser | Ship's Office |

The Wardroom is not a home workspace. It holds agents that are confirmed
online and have no active work.

## Agent State Model

Each adviser resolves to exactly one visual placement even when the underlying
agent has several concurrent turns.

| Confirmed state | Visual treatment |
|---|---|
| Online with no active turn | In the Wardroom |
| Working independently | At the home workspace |
| Collaborating | Moving to or present in the resolved collaboration workspace |
| In transit | Walking along a predefined route; status already reflects the destination work |
| Waking, stopped, offline, or unknown | On a personnel strip outside the ship |

Animation never delays or changes the authoritative state. A collaboration is
active as soon as the state resolver receives it, even while the sprite is
still walking visually.

When an agent has several active turns, placement precedence is:

1. an active collaboration with an explicit workspace;
2. another confirmed collaboration, using the most recently active turn;
3. the most recently active solo turn;
4. the Wardroom when the runtime remains confirmed online and idle;
5. the personnel strip otherwise.

The agent popover lists additional active work that does not control the sprite
placement.

## Collaboration and Movement Rules

Collaboration uses a context-led destination with an explicit override.

| Context | Default workspace |
|---|---|
| Operations or intelligence | C.I.C. |
| Navigation | Chart House |
| Command coordination or planning | Meeting Room |
| Reporting or daily routine | Ship's Office |
| Logistics | Supply Office |

An explicitly declared collaboration workspace wins over the context mapping.
This allows N2, for example, to leave DSE and collaborate in the C.I.C., Meeting
Room, Ship's Office, or another supported room.

The display must distinguish two claims:

- **collaborating with** requires a shared collaboration identifier and
  confirmed participants;
- **working in the same channel** may be derived from concurrent channel turns
  but must not be presented as collaboration.

Movement follows predefined room-to-room route segments. The visual layer does
not run general pathfinding. With reduced motion enabled, the sprite uses a
short fade between origin and destination instead of walking.

When work ends, an agent moves to another controlling active turn if one
exists. Otherwise, a still-online agent returns to the Wardroom.

## User Interaction

Clicking an agent opens a compact status card containing:

- adviser name and role;
- current state and elapsed time;
- current room and destination while walking;
- task or channel label when the viewer may access it;
- confirmed collaborators;
- the placement reason, such as "Operations collaboration";
- an **Open activity** action that reuses the existing agent-activity ingress.

Clicking a room shows its name, purpose, current occupants, and active
collaboration count. Keyboard focus and activation provide the same behaviour.
Status is never communicated by colour or animation alone.

The activity-opening path keeps the existing access controls. If the viewer
cannot open the underlying channel, the ship does not leak its name, messages,
or participant details.

## Architecture

The feature is isolated under `desktop/src/features/livingShip/`.

Suggested units are:

- `domain/shipLayout.ts` - room IDs, normalized coordinates, routes, home-room
  mappings, and collaboration-context mappings;
- `domain/resolveShipPlacement.ts` - pure placement and precedence rules;
- `livingShipStore.ts` - community-scoped normalized ship state and stable
  external-store snapshots;
- `useLivingShipState.ts` - adapter over profiles, managed runtime status,
  active turns, and observer state;
- `ui/LivingShipScreen.tsx` - responsive scene and selection state;
- `ui/AgentSprite.tsx` - visual state and reduced-motion movement;
- `ui/AgentStatusPopover.tsx` - accessible status and activity actions;
- `ui/RoomHotspot.tsx` - room occupancy and keyboard interaction;
- `assets/` - ship shell, room art, and sprite sheets.

The rendering layer consumes resolved state only. It does not parse raw ACP or
Nostr events.

### Existing Sources Reused

- owner-global observer ingestion and kind `24200` frames;
- `activeAgentTurnsStore` and its liveness/stale-turn handling;
- `agentWorkingSignal` for working-channel summaries;
- managed-agent runtime status and relay-agent profiles;
- `useOpenAgentActivity` for the existing activity panel;
- community reset handling in `resetCommunityState()`.

If `livingShipStore.ts` is implemented as a module-level singleton, it must
export `resetLivingShipState()` and register that reset with
`resetCommunityState()` so no positions or collaborators leak between
communities.

## Observer Collaboration Metadata

The existing owner-only observer frame gains optional collaboration fields on
the appropriate turn-start/activity payloads:

```text
collaboration_id
workspace
lead_pubkey
participant_pubkeys
```

`workspace` contains a stable room ID such as `cic`, `meeting_room`, or
`ships_office`; it never contains screen coordinates. The frontend owns the
coordinate mapping.

These fields carry no prompts, secrets, source evidence, tool parameters, or
detailed task content. Existing frames without them remain valid and resolve
to ordinary solo work. No new HTTP endpoint is introduced.

If the collaboration initiator does not declare a workspace, Command Adviser
applies the approved context mapping. If neither explicit metadata nor a
deterministic context is available, the agent remains at its home workspace
and is labelled working rather than collaborating.

## Error and Recovery Behaviour

- Unknown persona: keep the agent on the personnel strip with an unassigned
  role label; do not silently omit it.
- Unknown workspace: ignore the workspace value, keep the agent at its home
  workspace, and expose a non-sensitive diagnostic locally.
- Missing collaboration participants: do not claim collaboration; present the
  active turn as work.
- Observer-frame gap: reuse existing bounded liveness and stale-turn rules;
  animation freezes at the last confirmed transition until state resolves.
- Runtime stops during transit: cancel the visual route and move the agent to
  the unavailable personnel strip.
- Asset failure: preserve the status list and activity actions in a readable
  fallback panel even if ship art fails to load.
- Community switch: cancel all animation, clear selection, reset the store,
  and resolve the new community from fresh sources.

## Performance and Resource Boundaries

The MVP uses React, layered raster assets, and CSS transforms. It does not add a
game engine, physics system, video loop, general pathfinding, or render-time AI
generation.

Performance constraints are:

- at most eight primary adviser sprites;
- render on state changes rather than clock-driven scene reconstruction;
- elapsed-time text uses the existing low-frequency timer pattern;
- suspend animation and recurring visual work while the screen or application
  is hidden;
- load the ship assets only when the Ship screen is first opened;
- use stable references so unrelated observer activity does not rerender every
  room and sprite;
- preserve text zoom with rem-based labels and controls; pixel-art scaling is
  independent of readable UI text.

The feature adds no model calls and no backend polling.

## Accessibility

- Respect `prefers-reduced-motion` and the application's motion settings.
- Make every agent and room hotspot keyboard reachable.
- Provide visible focus, accessible names, and state descriptions.
- Do not depend on colour, sprite posture, or animation alone.
- Keep popover text on the existing rem-based typography scale.
- Provide the personnel/status list as a complete non-visual representation of
  the scene.

## Testing and Verification

### Unit Tests

- all eight home-workspace mappings;
- all context-to-room mappings;
- explicit workspace override precedence;
- online-idle Wardroom placement;
- waking, stopped, offline, and unknown placement outside the ship;
- collaboration versus same-channel wording;
- concurrent-turn precedence and fallback;
- unknown persona and workspace handling;
- community reset and stable snapshot behaviour.

### Component and Interaction Tests

- agent and room keyboard navigation;
- status cards and placement reasons;
- activity opening with accessible and inaccessible channels;
- reduced-motion transitions;
- failed-art fallback;
- scene scaling at supported desktop window sizes.

### Playwright Visual Tests

Capture distinct, animation-settled screenshots for:

1. every adviser idle in the Wardroom;
2. all advisers working independently;
3. an operations collaboration in the C.I.C.;
4. agents in transit between aft and forward modules;
5. waking and unavailable agents on the personnel strip;
6. the compact agent status card;
7. a reduced-motion transition.

Use the repository's animation wait helper before every screenshot. Scope each
capture to its intended subject and verify screenshot hashes are distinct
before posting them to a pull request.

### Installed-App Acceptance

The pull request remains draft until a real Command Adviser journey proves:

1. an online idle adviser appears in the Wardroom;
2. starting solo work moves the adviser to the correct home workspace;
3. an explicit multi-agent collaboration moves every participant to the same
   context-selected room and lists the correct collaborators;
4. clicking each participant opens the correct accessible activity;
5. completion returns agents to another active turn or the Wardroom;
6. stopping an agent removes it from the ship;
7. a community switch leaves no prior-community positions or collaborators;
8. animation stops while the Ship screen is hidden.

## MVP Boundary

The MVP includes the ship screen, complete silhouette, approved room modules,
eight adviser sprites, truthful state placement, deterministic movement,
collaboration metadata, click and keyboard interaction, activity integration,
reduced motion, tests, and installed-app acceptance.

The MVP deliberately excludes random wandering, weather, sea state, day/night,
sound, interactive furniture, historical playback, non-agent crew, multiple
ships, and procedurally generated rooms.

## Design Completion

The screen structure, room layout, roster, home workspaces, context rules,
truth boundary, architecture, performance boundary, and MVP scope are approved.
Pixel-level palette, furniture, and individual sprite details may be refined
during visual production without changing this contract. The continuous ship
outline and approved room blocking must remain intact.
