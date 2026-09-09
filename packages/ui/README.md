# Buzz UI

Open-source React components with Buzz's semantic roles, Inter typography, and
Lucide icons. Behavior comes from public Base UI primitives; calendar and resize
behavior come from DayPicker and react-resizable-panels. No internal package,
font CDN, Storybook server, or native Buzz bridge is required.

From the repository root:

```sh
. ./bin/activate-hermit
pnpm install --frozen-lockfile
pnpm --filter buzz-desktop-next dev
# Open http://localhost:1430/design
pnpm --filter @buzz/ui build
```

## Consume

```tsx
import '@fontsource-variable/inter/index.css';
import '@fontsource/jetbrains-mono/index.css';
import { Button, Field, Input } from '@buzz/ui';

<Field.Root name="name">
  <Field.Label>Name</Field.Label>
  <Input required />
  <Field.Error />
</Field.Root>
<Button variant="primary">Continue</Button>
```

The workspace export includes the source CSS; use a Tailwind 4 PostCSS pipeline.
For a consumer without Tailwind, build the package and use `@buzz/ui/dist` plus
`@buzz/ui/dist/styles.css`. Build output keeps React and behavior libraries external.
The package is intentionally private until package naming and release automation
are agreed; it is fully usable from this open-source workspace.

The stylesheet includes the shared theme and Tailwind reset. Load it once at the
application boundary. Set `class="dark"` on the document root for dark mode.
Font packages belong to the host, allowing self-hosted subsets without remote requests.

## Component contract

Compound families retain Base UI's API: `Dialog.Root`, `Dialog.Trigger`,
`Dialog.Portal`, `Dialog.Backdrop`, `Dialog.Popup`, `Dialog.Title`, and
`Dialog.Close`. Use `render={<Button />}` to skin triggers and closes. Supply a
Title and Description for dialogs and popovers. Label inputs with Field.Label;
label standalone controls directly. IconButton requires an accessible name.

Behavioral Root, Portal, and state helpers are direct exports, preserving generics,
refs, controlled state, and events. Styled parts merge both string and state-function
class names. The underlying libraries own focus management, keyboard navigation,
ARIA semantics, and dismissal. Caller-supplied labels and composition still matter.

Place `Tabs.Indicator` inside `Tabs.List` for a sliding selection pill.
Use `Tabs.List variant="glass"` for translucent chrome; it shares the same
selection behavior and motion as the default solid variant. Pointer
selection retargets the shared 120ms transition; keyboard focus and reduced motion
keep selection immediate. Panels update immediately. Toasts use the shared
180ms entrance and 120ms exit transitions, with no movement under reduced motion.

Button variants: primary, secondary, outline, ghost, danger, link. Sizes: sm, md,
lg. Loading disables duplicate activation while preserving the accessible name.
Color, type, geometry, and motion are roles; components never consume color ramps.
BarChart covers nonnegative categorical bars with a semantic data table, not a
full chart grammar. Carousel is manually advanced, without autoplay or drag.

Naming differences from shadcn: Menu is DropdownMenu; PreviewCard is HoverCard;
OTPField is InputOTP; Field.Label is Label; Toast replaces Sonner's role; BarChart
is the initial Chart treatment. Sheet composes Dialog. No shadcn API compatibility
or proprietary BlockUI package compatibility is implied.

See `../../desktop-next/OPEN_SOURCE.md` for provenance and `THIRD_PARTY_NOTICES.md`
for upstream notices. New source is covered by the repository's Apache-2.0 license.

Native scroll surfaces use thin scrollbars; custom ScrollArea bars share the
`size-scrollbar` token. Resize grips use `size-grip-length` (1rem) for their visible
length while retaining a larger pointer target. Table cells align to the reading
edge, including row and column headings.

## AI composer

`AIComposer` is a controlled multiline prompt surface. It requires nonblank text,
uses Enter to submit and Shift+Enter for newlines, and leaves IME composition and
other modifier combinations alone. It grows between the shared multiline bounds,
then scrolls with the system's thin scrollbar. Button and keyboard submissions use
one path; pending submissions disable duplicate sends.

```tsx
<AIComposer
  value={draft}
  onValueChange={setDraft}
  onSubmit={async (text) => {
    await acceptPrompt(text); // Host-owned request with timeout/retry policy.
    setDraft(""); // Clear only after acceptance; rejected requests retain the draft.
  }}
  onAttach={openFilePicker}
  context={({ disabled }) => <ContextChips disabled={disabled} />}
  controls={({ disabled }) => <ModelControls disabled={disabled} />}
  generation={isGenerating ? { onStop: stopGeneration } : undefined}
/>
```

The host supplies model/permission controls, context, and attachment behavior.
Slots receive `disabled` and must apply it to their controls. Rejections show an
inline retry message; the host must not clear the controlled draft before success.
While generation is active, the stop action stays available even if editing is
disabled. Optional `voice` supplies a labeled host callback; the component never
requests microphone access or uploads data. Authentication, permission enforcement,
model execution, file validation, speech capture, and network lifecycle belong to
the host. The catalog demonstrates these seams locally, including a sample voice
transcript and bounded attachment metadata; it sends nothing to a model.

Use `CardHeader` around a card title and description for an 8px gap at the default
root size, while preserving the larger spacing between the card's other sections.

## Generated responses

`GeneratedResponse` accepts unknown input and validates it before rendering. The
version 1 schema supports text, notices, metrics, task lists, tables, and action
lists. It caps serialized input at 64,000 characters, 20 blocks, 30 tasks per list,
50 table rows, 8 columns, and 6 actions per action list. IDs must be unique in their
scope, and action IDs are unique across the snapshot. Unknown fields and types fail.

The schema accepts no HTML, URLs, arbitrary styles, or component names. Text is
rendered through React escaping. The host supplies an own-property action allowlist;
unknown actions stay disabled. Rendering never dispatches an action. `state="streaming"`
and `pendingAction` prevent activation while a snapshot or request is in flight.
Invalid input renders an error and an optional host-provided retry callback.

The host owns streaming assembly, action authorization, persistence, retries, and
network errors. Pass a new immutable complete snapshot when it changes. The renderer
is a presentation boundary, not an agent execution engine or an authorization system.
