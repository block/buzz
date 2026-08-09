# Command Adviser Battle Rhythm and Plans

## Outcome

Add two first-class functions to the existing Command Adviser macOS
application:

- **Battle Rhythm** provides the authoritative operational calendar for the
  Fleet Activity Schedule (FAS), Longcast, Shortcast, recurring routines,
  reports, meetings, and manually entered activities.
- **Plans** provides deployment and major-activity project management through
  hierarchical tasks, dependencies, progress, critical-path calculation, and
  mission constraints.

Both functions are selectable directly from the application sidebar. They are
separate entities and views, linked only where planning tasks affect the
operational calendar. A Gantt task due date appears at the top of the relevant
calendar day as a linked all-day milestone; editing the task moves the
milestone.

This is a focused extension of the working Command Adviser product. It reuses
the existing Buzz event history, Command Team agents, Daily Command Brief,
RAG, Memory, Cloud-first/Local-first model routing, and native Apple
integration. It does not introduce a second application, planning daemon,
database, replication system, or separate agent framework.

## User Outcome

The Commanding Officer can:

- open **Battle Rhythm** or **Plans** directly from the left menu;
- view the ship's programme across year, month, week, and day horizons;
- enter operational events and recurring routines manually;
- import FAS, Longcast, and Shortcast documents from Word, Excel, or PDF;
- preview and correct an import before it changes the calendar;
- import a revised document without duplicating the previous version;
- publish approved calendar information one-way to a dedicated Apple
  Calendar;
- create a deployment plan with task hierarchy, owners, dependencies,
  progress, and due dates;
- see the calculated schedule critical path;
- connect defects and readiness conditions to mission requirements;
- see planning deadlines as linked all-day calendar milestones;
- receive useful warnings about contradictory source documents and likely
  missing prerequisites;
- ask the existing Command Team to analyse or propose changes to the Battle
  Rhythm and Plans; and
- receive Battle Rhythm, critical-path, and mission-constraint information in
  the Daily Command Brief.

The application remains advisory. Imports, proposed events, and agent changes
require explicit user approval.

## Representative Source Material

The design is informed by the supplied examples:

- `Long Term Planning Tool.xlsx` uses monthly, weekly, and daily programme
  bands with departmental overlays and port/sea colour states.
- `ANZAC NT15 Planning v7.xlsx` contains a FAS passage table, a long-term
  programme view, and a Gantt-style Navigation Department plan with WBS,
  owners, dates, duration, completion, and remaining work.
- `Shortcast 27 Mar - 16 Apr 15.docx` uses a chronological
  Time/Event/I-C/Remarks structure with recurring routines, briefs, training,
  watches, and one-off activities.
- `Daily Orders 7 Apr.pdf` shows a daily command view combining priorities,
  routine, timed events, responsibility, and remarks.
- `Presail Checklists.docx` demonstrates requirements anchored to operational
  events, including days or minutes before sailing and actions on arrival.
- `AFTP 4(K)` shows that plans and training requirements flow into operational
  planning artefacts such as the FAS, Shortcast, and Daily Orders.

The source documents remain user data and are not committed as repository test
fixtures. Automated tests use small synthetic equivalents; live acceptance
uses the local source files.

## Existing Substrate

The implementation will reuse:

- the Tauri 2 and React 19 Command Adviser desktop application;
- the existing sidebar and naval product identity;
- signed Buzz/Nostr events and the existing relay history;
- managed Command Team agents and their conversation workflows;
- the existing approval-gated workspace-action pattern;
- trusted-LAN RAG and Memory evidence;
- the persistent Cloud-first/Local-first model preference;
- Daily Command Brief scheduling and source-ledger behaviour;
- macOS Keychain and the native Apple privacy/permission boundary; and
- partial/degraded behaviour when an optional evidence source is unavailable.

New agent-facing operations follow the existing Buzz convention: define the
signed event contracts first, then expose the necessary read and proposal
operations through `buzz-cli`. The desktop application consumes the same
contracts rather than creating a private parallel API.

## Product Placement

The Command Adviser sidebar adds two independent destinations:

1. **Battle Rhythm**
2. **Plans**

Battle Rhythm is not a tab hidden inside Plans, and Plans is not a calendar
mode. Each destination retains its own navigation state. Links between them
open the corresponding source entity:

- selecting a projected Gantt milestone in Battle Rhythm opens its task in
  Plans;
- selecting a deployment activity in Battle Rhythm can open its linked plan;
- selecting a plan's mission-ready milestone can open the corresponding
  calendar date.

## Shared Concepts

### Battle Rhythm source

A `BattleRhythmSource` identifies the owner of imported information:

- stable source ID;
- source type: FAS, Longcast, Shortcast, or other approved schedule;
- display name;
- coverage start and end;
- document name and content hash;
- revision ID and prior revision ID;
- import timestamp;
- import status; and
- source-document reference.

Source identity is selected or confirmed during import. It does not rely only
on a filename, because revised documents may be renamed.

### Operational calendar event

A `BattleRhythmEvent` contains:

- stable event ID;
- title and description;
- event type, such as port, passage, exercise, maintenance, meeting, brief,
  report, routine, deadline, or other activity;
- start, end, all-day state, and time zone;
- recurrence and exception information where applicable;
- location;
- responsible owner and participants;
- remarks;
- approval and cancellation state;
- optional linked plan, task, mission requirement, or parent activity;
- ownership: manual or a specific source revision;
- original source location and extraction evidence; and
- signed creation and revision audit information.

Manual events do not belong to an imported source and cannot be removed by a
source re-import.

### Planning project

A `PlanningProject` represents one deployment, major activity, readiness
effort, or other bounded plan:

- stable project ID;
- title, purpose, and mission-ready date;
- status and overall progress;
- responsible owner;
- linked Battle Rhythm activities;
- planning assumptions; and
- signed creation and revision audit information.

### Planning task

A `PlanningTask` contains:

- stable task ID and project ID;
- WBS position and optional parent task;
- title, owner, status, and percentage complete;
- planned start, due date, and working duration;
- dependencies;
- optional fixed-date constraint;
- optional linked capability or mission requirement;
- notes and source evidence; and
- signed creation and revision audit information.

The first release supports finish-to-start dependencies, summary tasks, and a
configurable ship working-day calendar. Additional dependency types can be
added later without changing the core entity boundary.

### Mission constraint

A `MissionConstraint` records a defect, missing capability, readiness
condition, external dependency, or unresolved assumption that can affect a
mission requirement:

- stable constraint ID and project ID;
- description and owner;
- linked mission requirement, capability, task, or milestone;
- severity and current status;
- required resolution date where known;
- mitigation or disposition note;
- source evidence; and
- signed creation and revision audit information.

The first release can record that a matter is being considered as an
operational limitation or operational-risk issue, but it does not implement a
complete OPLIM or operational-risk assessment module.

### Calendar milestone projection

A Gantt task deadline is shown in Battle Rhythm as a derived all-day
milestone. It is not an independently editable `BattleRhythmEvent`.

The projection has a stable mapping to its underlying task so that:

- moving the task moves the milestone;
- completing the task updates the milestone's presentation;
- deleting the task removes the milestone;
- selecting the milestone opens the task; and
- one-way Apple publication can update the corresponding Apple event.

## Battle Rhythm Views

### Year / Longcast

The year view is a horizontally banded operational timeline rather than twelve
small conventional calendars. It shows:

- port and sea periods;
- passages;
- maintenance and training periods;
- exercises and deployments;
- major reports and decision points; and
- optional departmental or source overlays.

The default horizon is 12 months, with an extension to 24 months for Longcast
planning.

### Month

The month view shows:

- all-day programme activities;
- reports, briefs, meetings, and deadlines;
- projected Gantt milestones;
- conflict and assurance indicators; and
- source or owner colour filters.

### Week

The week view places all-day events and projected Gantt milestones at the top
of each day, followed by timed events. Overlapping events remain visible and
can be filtered by owner, source, or activity type.

### Day / Shortcast

The day view presents a ship-oriented chronological table:

- Time;
- Event;
- Responsible owner;
- Remarks; and
- source or status indicators.

It also supports day-routine labels such as alongside, cruising watches, or
defence watches without forcing those labels onto every individual event.

### Calendar controls

Battle Rhythm provides:

- **New Event**;
- **Import Document**;
- **Planning Review**;
- source, owner, type, and status filters;
- date navigation and Today;
- an Apple Calendar publication status; and
- event details containing source and revision history.

## Plans Views

### Project list

Plans opens to active and upcoming projects with:

- mission-ready date;
- overall progress;
- critical-path health;
- next due milestone;
- open mission constraints; and
- linked operational activity.

### Gantt

The Gantt view provides:

- hierarchical WBS tasks;
- owner, start, due date, duration, progress, and status;
- dependency links;
- summary tasks;
- working-day-aware bars;
- calculated total float;
- critical-path highlighting; and
- a visible mission-ready milestone.

Editing a task updates the Gantt and its derived calendar projection in one
operation.

### Critical path

The schedule critical path is calculated using task duration, dependencies,
working days, fixed-date constraints, and the mission-ready milestone.

The system:

1. validates the dependency graph and flags cycles;
2. performs forward and backward schedule calculations;
3. calculates total float;
4. identifies tasks with no available float as critical; and
5. recalculates when dates, durations, dependencies, or progress change.

Tasks missing the information required for a valid calculation are identified
as incomplete planning data rather than silently excluded.

### Mission constraints

Mission constraints are displayed beside the Gantt because schedule
criticality alone is insufficient to represent operational consequence.

For example:

1. a deployment requires seaboat operations;
2. the plan contains a `Seaboat capability available` milestone;
3. a seaboat davit defect blocks that milestone;
4. repair tasks connect the defect to the schedule;
5. the repair may become part of the calculated critical path; and
6. if the repair cannot be completed, the constraint remains visible until
   the mission changes, a mitigation is accepted, or the matter is deliberately
   moved into a future OPLIM or operational-risk process.

Completing an administrative checklist does not automatically resolve the
operational consequence of an open constraint.

## Import and Revision Workflow

### 1. Select

The user selects a Word, Excel, or PDF document and confirms:

- FAS, Longcast, Shortcast, or Deployment Plan;
- new source or revision of an existing source; and
- the proposed coverage period.

The application may suggest these values but does not decide them silently.
Scanned PDFs use OCR before structured extraction.

### 2. Extract and preview

The importer produces proposed calendar events or project tasks using a
shared structured contract. The preview shows:

- the original source location;
- extracted dates, times, owners, remarks, dependencies, and recurrence;
- uncertain or missing fields;
- source rows, cells, pages, or quoted locations; and
- corrections that the user can make before approval.

Extraction may use the selected model route, but no import requires the model
to write directly to the calendar or plan.

### 3. Compare revisions

For a revision, the system compares against the prior approved revision of the
same source and coverage period. It displays:

- added entries;
- changed entries;
- removed entries;
- unchanged entries; and
- identity matches that require confirmation.

After approval, the revision replaces only the entries owned by that source
within the approved coverage period. Manual events and events owned by other
sources remain untouched.

The update is atomic from the user's perspective. A failed import leaves the
previous approved revision active.

### 4. Planning Assurance Pass

Before approval, the proposed revision is checked for contradictions and
likely omissions.

#### Deterministic checks

Initial deterministic checks include:

- duplicate or overlapping source entries;
- FAS, Longcast, and Shortcast date conflicts for matched activities;
- activities outside the source's declared coverage period;
- invalid or contradictory time ranges;
- missing required fields;
- broken or cyclic planning dependencies;
- missing configured prerequisite events;
- relative-event rules such as preparations required a number of working days
  or minutes before sailing; and
- calendar milestones that no longer match their source tasks.

#### Knowledge-backed AI review

The selected Cloud-first or Local-first route may perform a second structured
review using:

- the proposed import and deterministic findings;
- applicable RAG doctrine;
- approved checklists and planning templates;
- relevant Memory outcomes;
- existing Battle Rhythm and Plans data; and
- previously approved ship planning patterns.

The review distinguishes:

- conflicting source;
- missing prerequisite;
- suspicious timing;
- possible omission; and
- unresolved ambiguity.

Each `PlanningFinding` contains evidence, rationale, confidence, affected
entities, and an optional proposed correction. Retrieved evidence is treated
as guidance rather than executable instructions. If no relevant doctrine is
found, the model may still provide its assessment using available evidence.

No finding creates or alters an event without user approval.

### 5. Pattern learning

An accepted correction can be saved as a reusable planning rule or template.
Repeated accepted corrections may prompt the user to promote a pattern, but
the application does not silently convert model output into a permanent rule.
Dismissed findings are retained as bounded feedback to reduce repeated noise.

### 6. Approve, publish, and retain

Approval:

- activates the new source or plan revision;
- applies selected assurance corrections;
- updates linked Gantt projections;
- reconciles the dedicated Apple Calendar; and
- retains the prior revision for history and rollback.

## Apple Calendar Publication

Command Adviser's internal Battle Rhythm is authoritative.

The application creates or selects a dedicated Apple Calendar, named
**HMAS Supply Battle Rhythm** by default. It publishes:

- approved operational calendar events; and
- approved Gantt due-date projections.

Publication is one-way:

- Apple Calendar changes do not modify Command Adviser;
- the publisher only manages events it created in the dedicated calendar;
- it does not read unrelated personal calendars into Battle Rhythm;
- a later reconciliation restores Command Adviser's authoritative title,
  timing, status, or deletion state; and
- publication failure leaves the internal Battle Rhythm usable and exposes a
  clear retry status.

The existing EventKit permission canary and fail-soft behaviour are extended
for write access. Permission denial degrades only publication.

## Adviser Integration

All Command Team agents receive read access to bounded Battle Rhythm and Plans
queries. Agents propose changes through signed proposal contracts and cannot
silently write approved data.

### Adviser responsibilities

- **Operations Adviser:** coordinates FAS, Longcast, and Shortcast activity,
  detects programme conflicts, and commissions the relevant advisers.
- **Plans Adviser:** establishes deployment projects, maintains dependencies,
  explains critical-path movement, and identifies missing planning data.
- **Navigation Adviser:** checks sailing, arrival, pilotage, passage-planning,
  and navigation-brief requirements.
- **Logistics Adviser:** identifies fuel, stores, maintenance, spares,
  port-service, and sustainment dependencies.
- **Maritime N2 Adviser:** uses future locations and activities to focus
  bounded regional monitoring.
- **Daily Routine Adviser:** supports recurring routines, meetings, and
  Shortcast preparation.
- **Reporting Adviser:** connects reports and returns to calendar deadlines.
- **Chief of Staff:** consolidates conflicts, constraints, dissent, and
  decisions requiring the CO.

For substantive planning advice, advisers seek applicable doctrine and cite it
when found. If no relevant doctrine is retrieved, they may still provide a
reasoned assessment from the available information.

### Conversation-driven planning

A discussion such as "we deploy through the Philippines in six months" can
cause Operations or the Chief of Staff to:

1. identify applicable doctrine and existing commitments;
2. propose a linked deployment project;
3. commission Plans, N2, Navigation, Logistics, and other relevant advisers;
4. produce draft events, tasks, constraints, and monitoring requirements; and
5. present the complete proposal for user approval.

Accepted discussion outcomes remain available through the existing Memory
path and can inform later Battle Rhythm reviews and Command Briefs.

## Daily Command Brief

Battle Rhythm and Plans become first-class brief sources. The brief includes:

- today's programme and Shortcast;
- upcoming briefs, reports, deadlines, and decisions;
- changes since the previous brief;
- slipping or overdue critical-path tasks;
- upcoming Gantt milestones;
- unresolved mission constraints;
- FAS, Longcast, and Shortcast conflicts;
- missing prerequisites;
- relevant 30-, 60-, and 90-day activities; and
- adviser recommendations with supporting evidence.

One failed import, source, agent, or AI assurance pass does not prevent a
partial useful brief from being generated.

## Failure Behaviour

The feature remains useful when optional components fail:

- a failed document extraction changes no approved data;
- a failed revision leaves the prior revision active;
- an unavailable model still permits manual events, calendar navigation,
  deterministic assurance, Gantt editing, and critical-path calculation;
- unavailable RAG or Memory reduces evidence but does not block planning;
- Apple permission or publication failure affects only the external mirror;
- invalid task dependencies are shown as planning errors without crashing the
  rest of the plan; and
- AI findings that fail schema validation are discarded rather than shown as
  trusted planning advice.

## Delivery Slices

### Slice 1: Usable Battle Rhythm

- sidebar destination and year/month/week/day views;
- manual event entry and recurrence;
- Word, Excel, and PDF import preview;
- source-owned revision replacement;
- deterministic import comparison;
- dedicated one-way Apple Calendar publication; and
- live macOS acceptance using representative documents.

### Slice 2: Usable Plans

- separate sidebar destination;
- projects, hierarchical tasks, and progress;
- Gantt rendering and editing;
- dependencies and working-day critical-path calculation;
- mission constraints;
- linked calendar milestone projections; and
- live macOS acceptance using a representative deployment plan.

### Slice 3: Intelligent Integration

- Planning Assurance Pass;
- reusable approved patterns;
- adviser read and proposal tools;
- conversation-driven planning;
- Daily Command Brief integration; and
- Cloud-first and Local-first acceptance.

Each slice must work through the actual signed macOS application before the
next slice begins.

## Acceptance

### Calendar and import

- import representative FAS, Longcast, and Shortcast structures from Word,
  Excel, and PDF;
- preserve source locations and uncertain-field review;
- re-import a changed source without duplicate entries;
- remove an omitted prior-source entry within the approved coverage period;
- preserve manual events and other source-owned events;
- retain and restore the previous approved revision;
- render usable year, month, week, and Shortcast day views; and
- keep the internal calendar usable when Apple publication is unavailable.

### Apple Calendar

- create or select the dedicated calendar;
- publish approved events and Gantt milestones;
- update moved or edited items without duplication;
- remove items deleted from the authoritative internal schedule;
- avoid modifying unrelated Apple calendars; and
- show a useful retry state after denied permission or transient failure.

### Plans

- import or create a representative hierarchical deployment plan;
- reject dependency cycles;
- calculate a known critical path and total float;
- recalculate after task duration or dependency changes;
- identify incomplete data that prevents a valid calculation;
- represent a seaboat davit defect as a blocker to a required capability;
- retain the unresolved constraint when the repair is not complete; and
- move a linked calendar milestone when its task due date moves.

### Planning Assurance

- flag a Shortcast sailing date that conflicts with the FAS;
- propose a missing configured pre-sailing activity;
- explain the source and rationale for each finding;
- operate deterministically when no model is available;
- use relevant RAG evidence without treating retrieved text as instructions;
- reject malformed AI findings;
- require approval before applying a proposed correction; and
- demonstrate useful detection on a known test set without excessive
  false-positive findings.

### Agent and brief integration

- allow the existing advisers to query relevant Battle Rhythm and plan data;
- preserve approval gates for proposed changes;
- show critical tasks, constraints, conflicts, and upcoming activities in a
  real Daily Command Brief; and
- produce useful partial output when one adviser or evidence source fails.

## Out of Scope

The first release does not include:

- two-way Apple Calendar synchronisation;
- personnel watchbill or duty-watch rostering;
- a complete OPLIM management system;
- a complete operational-risk assessment module;
- automatic acceptance of imports or agent proposals;
- unattended model-created permanent planning rules;
- a new external planning service or database;
- ship-control, navigation-control, or other operational-system integration;
  or
- guaranteed perfect interpretation of arbitrary unstructured documents.

These boundaries preserve a viable Command Adviser extension while leaving
stable links for later OPLIM and operational-risk functions if they become
necessary.
