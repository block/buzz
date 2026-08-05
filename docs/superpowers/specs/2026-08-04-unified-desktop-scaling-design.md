# Unified Desktop Scaling Design

## Goal

Buzz Desktop shall expose three persistent Appearance controls with a shared
75%-500% range:

- Interface scale changes the root UI scale.
- Chat text scale changes chat author and message text relative to the
  interface scale.
- Avatar scale changes identity avatars throughout the desktop application
  relative to the interface scale.

Changing any control must update the open application immediately without a
reload. The chosen value must survive restart.

## Scope

Avatar scale applies to rendered identity avatars in message and thread rows,
DM surfaces, navigation and sidebars, member and agent lists, search results,
projects, reminders, huddles, hover profile cards, and the profile panel.
Presence/status badges and thread rails are part of the same geometry and must
move and resize with their avatar.

The setting does not change uploaded image resolution, crop/capture canvases,
avatar source files, decorative artwork, emoji previews, or non-identity
icons.

## Architecture

### Preferences

The existing `createScalePreference` store remains the single persistence and
subscription mechanism. Interface, chat, and avatar preferences use the same
preset ladder:

`75, 90, 100, 110, 125, 150, 175, 200, 250, 300, 400, 500` percent.

Values read from storage are normalized to a supported preset. Existing saved
values remain valid. Invalid or out-of-range values fall back through the
existing normalization behavior.

Interface scale remains the base multiplier. Chat and avatar scale are
intentional relative multipliers, so their effective rendered size composes
with interface scale. The Appearance copy will state this relationship.

### Avatar metrics

A shared avatar metrics module owns:

- the supported scale range and presets;
- semantic base sizes for identity-avatar roles;
- conversion from a base size and current preference to a concrete CSS
  length;
- proportional status-dot and mask geometry;
- stable CSS custom properties needed by non-React layout code.

Components must not independently multiply hard-coded avatar dimensions.
Every avatar and every layout element that depends on it consumes the same
resolved metric. `transform: scale()` is not used because it does not reserve
layout space.

The shared `UserAvatar` component participates in Appearance scaling by
default. A narrowly named opt-out is allowed only for the excluded editor,
capture, and decorative surfaces. Specialized profile avatars consume the
same metric helpers instead of duplicating constants.

### Status indicators and masks

Status geometry is expressed as ratios of avatar size. The outer badge slot,
the visible dot, and the mask cutout are resolved together. The visible badge
fills its slot; it may not remain at a fixed Tailwind size while the slot grows.

The wrapper owns exactly the avatar box. The cutout changes only the visible
avatar shape and never adds external layout width or height. This removes the
empty crescent/void visible next to the status indicator at large scales.

### Dependent layout

Message gutters, thread indentation and rails, stacked avatars, row minimum
height, and profile-header spacing use resolved avatar metrics. Dense rows may
grow vertically at high avatar scale; avatars must not overlap text, adjacent
rows, controls, or scrollbars.

Appearance setting rows use wrapping, bounded labels, and a flexible slider
region so the controls remain reachable when interface scale is large. Panels
that cannot show all content at once remain scrollable; content must not be
clipped into an unreachable region.

## Accessibility and interaction

- Range controls expose their percentage through `aria-valuetext`.
- Keyboard operation and existing Cmd/Ctrl zoom shortcuts remain intact.
- Reset restores 100% and becomes disabled at the default.
- Status is not communicated by color alone; its accessible label remains.
- Interactive controls preserve at least a 44x44 CSS-pixel hit target where
  the surrounding component is interactive.
- Scaling updates layout without decorative animation or layout-shifting
  transforms.

## Verification

### Automated regression coverage

- Preference tests cover preset normalization, persistence, subscription, and
  the 500% upper bound for all three stores.
- Avatar metric tests cover 75%, 100%, 200%, and 500% for each semantic base
  size.
- Status geometry tests assert that slot, dot, cutout, and center remain
  proportional and inside the avatar box.
- Existing message/thread layout tests are updated to derive expected gutters
  and rails independently from the public scale contract.
- A desktop E2E test changes each slider, verifies its displayed value and
  persistence, and checks representative avatar surfaces.

### Rendered QA

Use the repository E2E mock bridge and `build:e2e`. Verify at minimum:

- Appearance settings at 100% and 500%;
- a channel/thread with avatars and rails at 75%, 200%, and 500%;
- hover profile and profile panel status indicators at 200% and 500%;
- representative dense list/sidebar/search surfaces at 500%;
- no relevant console errors, framework overlay, clipping, overlap, or
  unreachable controls in the tested flows.

Desktop and one constrained viewport are both checked. Any surface that cannot
be exercised through the mock bridge is reported as unverified rather than
assumed correct.

## Compatibility and constraints

The implementation preserves the user's current uncommitted work and does not
change relay, protocol, identity, media, or mobile behavior. It introduces no
new dependency. Existing local-storage keys stay unchanged. Production edits
are limited to desktop scaling stores, avatar components and their dependent
layout, Appearance settings, and focused tests.
