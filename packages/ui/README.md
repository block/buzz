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
# Open http://localhost:5173/design
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
