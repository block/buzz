# Command Adviser Project Execution V1

## Outcome

Extend the existing Command Adviser macOS application with a usable naval
calendar and project-execution workflow:

- Apple-style Battle Rhythm calendar views;
- a Kanban view over the existing Plans task model;
- durable HOD delegation and printable sync packs;
- reusable, routine-aware operational playbooks;
- drag-and-drop scheduling with reviewed recalculation; and
- hybrid AI task execution that produces linked Word, PowerPoint, Excel, and
  PDF artefacts.

This is a focused first version intended for installation and user refinement.
It extends the existing Battle Rhythm and Plans implementation rather than
introducing another project-management application, service, database, or task
authority.

## Product Boundary

Command Adviser remains the authoritative command-side planning tool. The
existing Buzz relay and signed-event history remain the only planning
persistence layer.

OpenProject is a behavioural and architectural reference for work packages,
Kanban boards, scheduling, and drag-and-drop interaction. Its GPL-3.0 source is
not copied into the Apache-2.0 Buzz fork. No OpenProject server or database is
introduced.

The first version is a CO/HOD coordination system, not a ship-wide tasking
portal. The CO synchronises with the:

- Executive Officer (XO);
- Marine Engineering Officer (MEO);
- Weapons Electrical Engineering Officer (WEEO); and
- Supply Officer (SO).

HODs delegate work through the normal chain of command. Ship's-company members
do not require individual Command Adviser accounts in V1.

## Existing Substrate

The design reuses:

- the Tauri 2 and React 19 Command Adviser application;
- the current Battle Rhythm Day, Week, Month, and Year routes;
- reviewed FAS, Longcast, and Shortcast imports;
- one-way Apple Calendar publication;
- the existing Plans project, task, dependency, Gantt, critical-path, mission
  constraint, and calendar-milestone contracts;
- signed Buzz events and relay queries;
- managed Command Team agents;
- Cloud-first and Local-first model routing;
- local RAG and Memory evidence;
- Daily Command Brief planning evidence; and
- the existing approval and inline-error patterns.

## Battle Rhythm Calendar

Battle Rhythm adopts a conventional Apple Calendar-style structure while
retaining Command Adviser's naval visual identity.

### Headers and navigation

- Day: `Wednesday, 29 July 2026`.
- Week: `27 July - 2 August 2026`.
- Week columns: `MON 27 JUL`, `TUE 28 JUL`, and equivalent.
- Month: `July 2026`, with weekday headings and a numeral in every day cell.
- Year: `2026`, with twelve conventional month grids.
- All views retain Today, previous, and next controls.
- Selecting a month in Year opens Month.
- Selecting a date in Year or Month opens Day.
- Today receives a consistent high-contrast accent.

All entered and displayed times use 24-hour notation.

The header shows the effective Ship Time and routine, for example:

`Ship Time: AEST (UTC+10) - Alongside Routine`

### Year and programme

Year provides the conventional twelve-month calendar requested by the user.
The existing operational FAS/Longcast view remains available through a
`Programme` toggle within the same Year horizon. Programme retains port, sea,
maintenance, exercise, deployment, report, and decision bands.

### Dragging calendar items

Manual events and approved playbook tasks can be repositioned by drag and
drop.

Imported FAS, Longcast, and Shortcast events remain source-controlled.
Dragging an imported item creates a proposed local adjustment for review; it
does not silently rewrite its approved source revision.

## Ship Routine and Time

### Routine authority

The approved FAS state determines scheduling availability for each date:

- Alongside: Monday-Friday, 0800-1600.
- At sea: a 24-hour cycle, Monday-Saturday.
- Sunday Sea: work may be scheduled from 1200 onward; 0000-1159 is excluded.

If the approved FAS does not cover a required date, the scheduler continues
using the most recent known routine and marks the assumption for review.

### Ship Time

Ship Time defaults to the IANA timezone `Australia/Sydney`.

Date-effective timezone changes arrive as approved Shortcast events. The
scheduler applies each change from its effective time onward. Calendar and
Plans headers display the active timezone.

Stored instants retain enough timezone information for audit and correct
one-way Apple Calendar publication.

## Unified Plans Workspace

Each project uses one underlying task set shown through:

- Board;
- Gantt;
- Work Breakdown;
- Constraints; and
- Playbooks.

Editing a task through any view updates the same signed task. Its Gantt bar,
critical-path calculation, Kanban card, and Battle Rhythm milestone therefore
remain consistent.

### Kanban

The default status columns are:

1. Planned
2. Ready
3. In Progress
4. Waiting
5. For Review
6. Complete

Dependencies determine whether a planned task is ready. Moving a card between
columns changes task status. Moving a task in Gantt or Battle Rhythm changes
its schedule. Kanban status movement does not unexpectedly change dates.

Each card shows:

- title and project;
- department or position owner;
- optional named individual;
- optional AI adviser;
- due date and time in Ship Time;
- dependency and missing-input indicators;
- critical-path marker;
- playbook source;
- generated-output state; and
- overdue or command-attention warnings.

Drag failures restore the prior card state and show the relay error inline.

## Assignment and HOD Sync

Task ownership supports:

- a durable department or position;
- an optional named individual; and
- an optional AI adviser as producer.

Position or department is the default so playbooks survive postings and
personnel changes. A named individual adds accountability when warranted.

Default HOD groupings are XO, MEO, WEEO, and SO. Other departments and
positions are configurable.

### HOD Sync Pack

Plans can generate:

- one combined printable PDF; and
- a separate printable task list for each HOD.

Tasks are ordered by:

1. overdue;
2. critical path; and
3. due date.

Each list includes status, due date and time, dependencies, command decisions
required, checkboxes, and space for handwritten notes.

## Playbooks

A playbook is a reusable operational preparation template anchored to a
Battle Rhythm event or project milestone.

An initial Pre-Departure example may contain:

- Navigation plan briefed;
- departure pilotage briefed;
- passage plan promulgated;
- OPTASK RAS sent;
- securing for sea rounds completed;
- mission-essential stores embarked;
- engineering readiness confirmed; and
- personnel and administrative checks complete.

### Playbook task template

Each template task records:

- title and instructions;
- timing before or after the anchor;
- expected working duration;
- dependencies;
- default department or position;
- optional AI adviser;
- required output type;
- whether it may be automatically rescheduled;
- whether a placed task is locked; and
- linked critical-path, capability, or mission requirement information.

Playbooks are versioned. New, duplicate, revised, and retired templates do not
rewrite projects already using an earlier revision.

### Applying a playbook

Command Adviser:

1. selects the anchor and its Ship Time;
2. reads the approved FAS routine for each affected date;
3. schedules work backwards or forwards through valid periods;
4. honours dependencies and locked dates;
5. calculates the resulting critical path;
6. presents a complete preview; and
7. applies the tasks only after approval.

### Rescheduling

When an anchor moves, Command Adviser previews a reflow:

- incomplete, unlocked playbook tasks move to valid working periods;
- completed tasks remain unchanged;
- locked tasks retain their dates;
- affected dependants are listed;
- critical path and float are recalculated;
- moved Battle Rhythm milestones are shown; and
- invalid routine periods are identified.

The new schedule is persisted only after approval.

Direct Gantt or calendar dragging uses the same preview-and-apply mechanism.

## AI Task Execution

A task can be assigned to an AI adviser alongside its organisational owner.

Execution modes are:

- Run now;
- Scheduled; and
- Hybrid.

Hybrid is the default. The user can start work manually at any time.
Otherwise, an unstarted task is commissioned one hour before its due time.
Planning tasks support an optional due time in addition to the existing due
date. A date-only AI task uses 1600 Ship Time as its visible default due time
and is therefore commissioned at 1500 unless the user changes it.

### Execution context

Before running, the adviser receives bounded:

- project purpose and mission-ready date;
- task instructions;
- dependency state and dependency artefacts;
- Battle Rhythm and FAS context;
- effective Ship Time and routine;
- constraints and critical-path information;
- relevant adviser discussions and Memory;
- relevant RAG doctrine and evidence; and
- required output format.

Doctrine is sought where relevant but is not a hard execution gate.

### Incomplete dependencies

Incomplete dependencies do not block execution. The adviser proceeds with
available information and records:

- missing inputs;
- assumptions;
- affected findings or document sections; and
- checks required from the user.

The task remains visibly flagged for dependency review.

Successful AI work moves to For Review rather than directly to Complete.

### Background scheduling

A small Command Adviser background scheduler starts at login and reuses the
existing agent and model-routing infrastructure. It is part of the macOS
application and does not introduce another server.

If the Mac was asleep or shut down at the scheduled time, the task starts when
Command Adviser resumes and records a late-start warning. If no model route is
available, the task remains queued with a retry action.

## Generated Artefacts

Supported outputs are:

- an in-app task response;
- Word `.docx`;
- PowerPoint `.pptx`;
- Excel `.xlsx`; and
- printable PDF.

### Storage

The preferred root is a project-specific Command Adviser folder in iCloud
Drive so outputs are available from the user's phone.

If iCloud is unavailable, generation continues into a local fallback folder.
The artefact is marked `Pending iCloud publication` and can be retried when
iCloud returns.

Each artefact record contains:

- Open and Reveal in Finder actions;
- actual saved path;
- file type and creation time;
- producing agent and model route;
- source task and project;
- short summary;
- missing-input warning; and
- iCloud publication state.

If rich-document generation fails, the adviser's text result is retained and
the task reports the document-generation error.

## Contracts

The existing planning contracts are extended without discarding existing
projects:

- `PlanningAssignee`
- `Playbook`
- `PlaybookRevision`
- `PlaybookTaskTemplate`
- `ProjectWorkingSchedule`
- `ShipRoutinePeriod`
- `TaskExecution`
- `TaskArtifact`
- `HodSyncPack`

Contract version migration keeps current projects and tasks readable. New
signed event kinds follow the existing Buzz/Nostr patterns and relay admission
rules. Agent reads and proposals use `buzz-cli` rather than a private HTTP
surface.

## Error Handling

- Invalid dependency cycles are rejected with affected tasks identified.
- Failed drag-and-drop writes restore the prior visual state.
- Missing FAS coverage becomes a visible assumption.
- Failed AI execution remains queued and retryable.
- Failed document generation preserves text output.
- iCloud failure uses the local fallback and exposes publication state.
- Imported source records cannot be silently changed through dragging.
- Late background starts and unavailable model routes remain visible.
- No failed operation is presented as complete.

## Delivery Checkpoints

### 1. Calendar refinement

- Apple-style labels and navigation;
- 24-hour time;
- Ship Time and routine display;
- conventional Year view; and
- Programme toggle.

Install and exercise the real macOS application before continuing.

### 2. Kanban and delegation

- unified Board/Gantt/task model;
- HOD and named-individual assignment;
- status drag and drop; and
- combined and per-HOD printable sync packs.

Install and exercise the real macOS application before continuing.

### 3. Playbooks and scheduling

- playbook editor and revisioning;
- FAS-aware work periods;
- Shortcast timezone changes;
- anchor-based scheduling;
- task locks and reviewed reflow; and
- critical-path and calendar updates.

Install and exercise the real macOS application before continuing.

### 4. AI execution and artefacts

- manual/scheduled hybrid execution;
- dependency-aware context;
- background catch-up;
- Word, PowerPoint, Excel, and PDF generation;
- iCloud-first storage and local fallback; and
- clickable artefact links.

Install and exercise the real macOS application before refinement.

## Verification

Automated and live acceptance cover:

- labels and navigation in Day, Week, Month, and Year;
- 24-hour time and Sydney default;
- date-effective Shortcast timezone changes;
- Alongside, At Sea, and Sunday Sea scheduling;
- weekend avoidance for a Monday sailing playbook;
- anchor movement, locked tasks, completed tasks, and dependency reflow;
- Kanban status movement and write-failure rollback;
- HOD grouping and printable output;
- AI execution with complete and incomplete dependencies;
- missed scheduled execution after sleep or shutdown;
- iCloud availability and disconnected local fallback;
- creation and opening of Word, PowerPoint, Excel, and PDF outputs;
- upgrade compatibility with existing Battle Rhythm and Plans events; and
- the complete installed macOS user journey at every checkpoint.

## V1 Acceptance

V1 is complete when the user can:

1. read all calendar horizons without inferring the date or year;
2. create a project and manage its tasks through Kanban, Gantt, and table
   views;
3. assign work to an HOD, position, optional individual, or AI adviser;
4. print an HOD sync pack;
5. apply and reschedule a Pre-Departure playbook around a sailing event;
6. see FAS routine and Shortcast Ship Time affect task scheduling;
7. run or automatically commission an AI task;
8. open the resulting artefact from its task; and
9. recover visibly from relay, model, document, or iCloud failure.

Further capabilities are driven by user experience with this installed first
version rather than additional speculative design.
