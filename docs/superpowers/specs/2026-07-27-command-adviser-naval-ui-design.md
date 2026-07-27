# Command Adviser Naval UI Refresh

## Outcome

Transform the working Buzz-based macOS client into a clearly identifiable
**Command Adviser** application for HMAS Supply. The refresh must make the Daily
Command Brief easier to use, apply restrained naval visual language, replace
realistic adviser portraits with symbolic naval identities, and remove visible
Buzz branding from the primary user journey.

This is a focused product-surface change. It must not redesign or gate the
working model router, adviser orchestration, Apple connectors, RAG, Memory,
signed event persistence, or fail-soft behaviour.

## Visual Direction

The selected source of truth is the first concept presented on 27 July 2026:
the dark-navy **Quarterdeck Brief** direction. Its defining characteristics are:

- a deep navy and charcoal command-dashboard shell;
- restrained brass/gold accents for primary actions and attention;
- an official HMAS Supply badge used unaltered;
- a shallow, atmospheric photograph of HMAS Supply rather than a large
  decorative hero;
- compact operational cards with strong information hierarchy;
- generous enough spacing to remain calm and legible;
- no imitation instrument panels, decorative military jargon, or excessive
  animation.

The application must continue to support the existing light/dark theme
mechanism where required by shared Buzz surfaces, but the Command Adviser
console itself is designed first for the dark naval treatment.

## Product Identity

The user-facing macOS product name is **Command Adviser**.

Update:

- Tauri `productName`;
- macOS bundle display/name strings;
- main window title;
- menu-bar and Dock identity;
- About text;
- DMG/install presentation;
- microphone, camera, Calendar, Reminders, and Notes permission descriptions;
- user-facing desktop copy in the Command Adviser journey;
- application icon assets.

Preserve the existing bundle identifier, Keychain service names, storage keys,
deep-link scheme, executable/crate names, protocol names, and internal Buzz
identifiers unless a visible string requires changing. This avoids invalidating
macOS privacy grants, Keychain credentials, saved application state, signed
events, or existing links.

The app icon will be an original, simplified navy-and-gold Command Adviser mark:
a ship silhouette combined with a compass/command motif. It must remain legible
from 16 px through the macOS 1024 px icon and must not present itself as an
official Royal Australian Navy application. The official HMAS Supply badge is
reserved for the in-app ship identity treatment.

## Official Ship Assets

Use the official Royal Australian Navy HMAS Supply page as the source:

- Ship page:
  <https://www.navy.gov.au/capabilities/ships-boats-and-submarines/hmas-supply-ii>
- Badge:
  <https://www.navy.gov.au/sites/default/files/styles/scale/public/2024-02/HMAS-Supply_badge.png?itok=onQUVLu_>
- Ship photograph:
  <https://www.navy.gov.au/sites/default/files/styles/landscape_large/public/media-gallery/2023-11/HMAS-Supply_20210115ran8555536_0071_0.jpg?h=82f92a78&itok=wJq65_lm>

Store local, optimised copies in the desktop application so the visual identity
does not depend on internet access. Keep the badge unaltered. The ship
photograph may be cropped and tonally treated by the UI, but the source image
itself remains unmodified. Add a concise asset attribution file and do not imply
Royal Australian Navy or Department of Defence endorsement.

## Adviser Identities

Replace realistic human avatars with one consistent set of circular symbolic
naval insignia:

| Adviser | Symbol |
| --- | --- |
| Chief of Staff | command star and anchor |
| Operations Adviser | radar plot |
| Navigation Adviser | sextant |
| Daily Routine Adviser | ship's bell |
| Reporting Adviser | clipboard and returns |
| Plans Adviser | charted course and waypoint |

Use a cohesive icon family wherever suitable. If the chosen library lacks a
recognisable sextant or other specialist symbol, create that missing item as a
small raster asset in the same line-art style rather than mixing in realistic
portraits, emoji, or improvised CSS/SVG drawings.

Each insignia uses the adviser label in accessible text. Meaning must not depend
on colour alone.

## Command Console

The console header combines:

- HMAS Supply badge and ship identity;
- `COMMAND ADVISER`;
- `HMAS SUPPLY · A195`;
- the motto `STRENGTHEN THE SHIELD`;
- a shallow ship image band;
- the existing Cloud first / Local first model toggle;
- a clear `Generate brief` or `Refresh brief` primary action.

The routing toggle remains globally persistent and locked during an active
brief exactly as it works now. Source and model health remain available, but
they move beneath the command content rather than occupying the primary visual
position.

The adviser team appears as compact specialist cards or a single team strip
using the symbolic identities. Cards show meaningful operating state such as
ready, contributing, unavailable, or completed. They do not add decorative
rank, medals, or fictional personal biographies.

## Daily Command Brief

The brief leads with what the Commanding Officer needs to know:

1. **Decisions and approvals required**
2. **Today at a glance**
3. **Operational priorities and risks**
4. **Navigation considerations**
5. **Daily routine and calendar**
6. **Reports and returns due**
7. **30 / 60 / 90-day outlook**

The current validated brief contract remains authoritative. The UI reshapes the
same content into this reading order and must continue to surface limitations,
missing inputs, stale information, and dissent where present.

Presentation rules:

- decisions and urgent risks are visually prominent;
- actionable items lead with the action, owner/time context, and consequence;
- sections without usable information show a concise honest empty/degraded
  state;
- adviser identity is secondary metadata, not the organising principle;
- dense source lists do not interrupt the main reading flow;
- long specialist contributions may be expanded when detail is needed.

## Evidence and System Status

Preserve citations, retrieval timestamps, source freshness, provider/model
provenance, generation audit ID, and source health. Present them in a collapsed
**Evidence and system status** section after the command content.

Inline citation markers remain attached to factual claims where supported. The
collapsed section supplies the full source ledger, adviser contributions,
provider route, and connector health. A degraded connector must remain visible
and truthful without displacing the useful portions of the brief.

## Interaction and Accessibility

- Keep the primary briefing flow usable by keyboard.
- Preserve visible focus states and semantic headings.
- Keep readable text on the existing rem-based type scale.
- Maintain sufficient contrast in both the navy shell and status treatments.
- Respect reduced-motion preferences.
- Do not use colour, icon, or hover state as the only carrier of meaning.
- Keep existing brief generation, cancellation, scheduling, routing, citation
  navigation, and disclosure interactions functional.

## Implementation Boundaries

In scope:

- Command Console and Daily Command Brief presentation;
- adviser symbolic identity components;
- official ship image/badge assets and attribution;
- user-visible branding;
- macOS display identity, icon, and packaging presentation;
- focused unit, E2E, screenshot, build, bundle, and identity verification.

Out of scope:

- changing adviser prompts or output contracts;
- changing model order, credentials, or fallback policy;
- changing RAG or Memory endpoints;
- changing Apple data access;
- changing signed persistence or the relay;
- renaming internal Buzz crates, binaries, storage keys, deep links, or bundle
  identifier;
- Phase 5 workspace actions or deferred RAG 2.0 work.

## Test and Acceptance Criteria

The refresh is accepted when:

1. The primary console reproduces the selected Option 1 visual direction at the
   current desktop reference viewport and remains usable at supported window
   sizes.
2. All six advisers use the approved symbolic naval identities; no realistic
   portrait avatars remain in the Command Adviser surfaces.
3. A populated brief reads in the approved decision-first order.
4. Evidence and system status are collapsed by default, remain discoverable,
   and preserve citations, limitations, dissent, freshness, provider, and
   connector information.
5. Existing Cloud first / Local first behaviour and active-run locking remain
   unchanged.
6. Existing Apple, RAG, Memory, signed-publish, fail-soft, scheduling, and
   cancellation paths retain their automated coverage.
7. Finder, Dock, menu bar, window title, About presentation, permission prompts,
   built `.app`, and DMG present the product as **Command Adviser**.
8. The existing bundle identifier, Keychain access, privacy permissions,
   persisted settings, and deep links continue to work.
9. No primary Command Adviser screen contains visible Buzz branding. Internal
   diagnostics may retain Buzz implementation names where technically useful.
10. Desktop lint, unit tests, Command Console E2E tests, screenshot/design QA,
    Tauri tests, release app build, and DMG build pass before handoff.

