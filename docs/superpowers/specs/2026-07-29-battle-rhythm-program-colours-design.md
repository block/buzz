# Battle Rhythm Ship Program Colours Design

**Date:** 29 July 2026  
**Status:** Approved  
**Scope:** Command Adviser Battle Rhythm presentation

## Purpose

Make the ship's broad program immediately readable in calendar views without
adding another field to every event or maintaining a worldwide port database.

## Classification

Colour classification applies only to all-day events:

- a location containing `Sea` or `At Sea`, matched case-insensitively as a
  word, is **at sea** and uses blue;
- any other non-empty location is **in port/alongside** and uses yellow; and
- an event without a location remains neutral.

FBE means Fleet Base East in Sydney. FBW means Fleet Base West at HMAS
Stirling/Garden Island in the Fremantle–Rockingham area. Both therefore
classify as in-port locations. These aliases are also recorded in Memory MCP.

The classifier is deterministic and presentation-only. It does not rewrite the
event, infer a port from the title, or change the FAS-derived ship routine.

## Calendar presentation

Day, Month, Week, and Year use the same event-colour helper so the meaning is
consistent:

- yellow: all-day in-port/alongside event;
- blue: all-day at-sea event; and
- existing neutral treatment: timed events or unclassified all-day events.

In Week view, the existing `All-day activities` area becomes a seven-column
lane. Each all-day event appears once, clipped to the displayed week and
spanning the calendar days it overlaps. All-day events are no longer repeated
inside each daily column. Timed events and plan milestones remain in their
existing daily columns.

Overlapping all-day events are displayed on separate rows so they do not cover
one another. Every event remains selectable for editing.

## Verification

Automated coverage will prove:

- FBE, FBW, Sydney, Fremantle, and arbitrary non-empty port locations classify
  as yellow for all-day events;
- `Sea` and `At Sea` classify as blue without misclassifying unrelated words;
- timed events retain the neutral treatment;
- a multi-day all-day event renders once in Week view with the correct span;
- daily columns no longer repeat all-day events; and
- Day, Month, Week, and Year apply consistent colours.

The signed macOS app will then be installed and checked using the user's
persisted 2027–2028 ship program without creating or modifying an event.
