/** States reachable in the live examples, rather than simulated screenshots. */
export const STATE_GUIDES: Record<string, readonly string[]> = {
  "ai-composer": [
    "Start empty, then write a multiline draft with Shift+Enter.",
    "Add or remove context and attachments; choose a model and action policy.",
    "Send to see generation and Stop. Simulate a send failure to inspect recovery.",
  ],
  buttons: [
    "Compare all six emphasis variants and the three sizes.",
    "Inspect disabled and loading actions alongside enabled buttons.",
    "Hover, press, or Tab to a button to compare pointer and keyboard feedback.",
  ],
  input: [
    "Compare empty, filled, invalid, read-only, and disabled fields below.",
    "Tab into the editable field and type to inspect its focus and content states.",
  ],
  textarea: [
    "Compare empty, filled, invalid, read-only, and disabled writing areas.",
    "Write multiple lines and resize the editable example.",
  ],
  "input-group": [
    "Type a query and inspect how the leading icon and shortcut stay aligned.",
    "Tab into the input to see the shared focus treatment.",
  ],
  "form-field": [
    "Submit empty or malformed email to inspect validation.",
    "Enter a valid email and submit to see confirmation.",
  ],
  checkbox: [
    "Toggle the live choice with a click or Space.",
    "Compare unchecked, checked, indeterminate, and disabled states below.",
  ],
  "checkbox-group-fieldset": [
    "Select several channels, then deselect them independently.",
    "Tab between choices and use Space to change selection.",
  ],
  "radio-group": [
    "Select either visibility option.",
    "Use arrow keys to move the single selection within the group.",
  ],
  switch: [
    "Switch the live setting on and off with a click or Space.",
    "Compare on, off, and disabled examples below.",
  ],
  slider: [
    "Drag the thumb or use arrow keys to adjust the value.",
    "Use Home and End to inspect the minimum and maximum.",
  ],
  "number-field": [
    "Type a number, or use the increment and decrement actions.",
    "Try the bounds of 1 and 12 to inspect unavailable step actions.",
  ],
  "input-otp": [
    "Enter a six-digit code; inspect the active digit as focus advances.",
    "Paste a code, or use Backspace to edit earlier digits.",
  ],
  combobox: [
    "Open the choices, then type to filter them.",
    "Search for a nonexistent project to see the empty state.",
    "Use arrow keys and Enter to select; Escape dismisses the popup.",
  ],
  autocomplete: [
    "Type a known project to inspect suggestions.",
    "Enter a new project name to retain free text.",
    "Use arrow keys to explore suggestions and Escape to dismiss.",
  ],
  command: [
    "Type to filter actions or show an empty result.",
    "Choose with Enter or a click; filtering alone does not execute an action.",
  ],
  calendar: [
    "Select or clear a date and navigate between months.",
    "Compare today and the selected day, including keyboard focus.",
  ],
  dialog: [
    "Open the dialog to inspect its title, description, and action.",
    "Tab through the modal, then dismiss with Done or Escape to restore trigger focus.",
  ],
  "alert-dialog": [
    "Open the decision and cancel with Keep draft.",
    "Confirm deletion to see the local result, then Restore.",
  ],
  sheet: [
    "Open the side surface and inspect its content at different widths.",
    "Dismiss with Close details or Escape to return focus.",
  ],
  drawer: [
    "Open the bottom surface and inspect its anchored position.",
    "Use Continue building or Escape to dismiss.",
  ],
  popover: [
    "Open the anchored share options.",
    "Dismiss with Got it, Escape, or a click outside.",
  ],
  tooltip: [
    "Hover the trigger or focus it with the keyboard.",
    "Move away or press Escape to dismiss the help.",
  ],
  "hover-card": [
    "Hover the team link to inspect its preview.",
    "Move away to close; essential link text stays on the page.",
  ],
  "dropdown-menu": [
    "Open the menu and explore the highlighted item with arrow keys.",
    "Choose an action to see the local result, or Escape to dismiss.",
  ],
  "context-menu": [
    "Right-click the target, or focus it and press Shift+F10.",
    "Choose Copy link or Mark as unread to inspect the result.",
  ],
  select: [
    "Open the choices and inspect the selected indicator.",
    "Use arrow keys and Enter to choose another digest frequency.",
  ],
  toast: [
    "Show a notification to inspect its entrance and status text.",
    "Show several to inspect stacking; dismiss with the close action.",
  ],
  accordion: [
    "Expand and collapse each heading.",
    "Use Tab and Enter or Space to compare keyboard interaction.",
  ],
  collapsible: [
    "Reveal the implementation notes and hide them again.",
    "Focus the trigger to inspect its expanded and collapsed semantics.",
  ],
  tabs: [
    "Switch between Overview, Activity, and Files to inspect the sliding indicator.",
    "Use arrow keys to focus a tab, then Enter to select it.",
    "Compare the solid and glass tracks and the disabled tab below.",
  ],
  toggle: [
    "Press to pin, then press again to unpin.",
    "Use Space to compare the same states with the keyboard.",
  ],
  "toggle-group": [
    "Switch the timeline scale between day, week, and month.",
    "Inspect selected and unselected choices with keyboard navigation.",
  ],
  toolbar: [
    "Use arrow keys to move among the grouped actions.",
    "Activate an action to inspect its local status result.",
  ],
  menubar: [
    "Open Project or View, then move among their menu items.",
    "Choose an action or dismiss with Escape.",
  ],
  "navigation-menu": [
    "Open Foundations to reveal its nested destinations.",
    "Follow a link to inspect navigation into a foundation page.",
  ],
  breadcrumb: [
    "Compare linked ancestors and the current location.",
    "Tab to an ancestor and follow it back to the catalog.",
  ],
  sidebar: [
    "Inspect the current destination alongside the other links.",
    "Tab through the destinations and follow a link.",
  ],
  pagination: [
    "Move through pages 1 to 3.",
    "Compare the current page and the disabled Previous or Next action at each bound.",
  ],
  card: [
    "Inspect the label, grouped heading, body, and action as one composition.",
    "Switch density to compare panel insets and heading spacing.",
  ],
  badge: [
    "Compare neutral, accent, success, warning, danger, and information tones.",
    "Compare the text-only badges with the icon and text success badge.",
  ],
  alert: [
    "Compare information, success, warning, and failure messages.",
    "Switch theme to inspect text and fill pairings.",
  ],
  avatar: [
    "Compare initials and the agent icon fallback.",
    "Switch theme and density to inspect identity at the shared control size.",
  ],
  progress: [
    "Compare empty, in-progress, and complete values below.",
    "Inspect both the visual fill and the accessible value.",
  ],
  meter: [
    "Compare low, medium, and full measurements below.",
    "Inspect the value label alongside the fill.",
  ],
  skeleton: [
    "Inspect the loading region and its differently sized placeholders.",
    "Enable reduced motion in your system to remove the pulse.",
  ],
  spinner: [
    "Inspect the named loading indicator.",
    "Enable reduced motion in your system to remove rotation.",
  ],
  "empty-state": [
    "Inspect the purpose, explanation, and next action together.",
    "Follow the action to see a populated composition.",
  ],
  separator: [
    "Inspect the boundary between two related text regions.",
    "Switch theme to compare the quiet divider treatment.",
  ],
  table: [
    "Inspect column headings, left alignment, and status cells.",
    "At a narrow width, scroll within the table rather than the whole page.",
  ],
  chart: [
    "Compare category labels, values, and proportional bars.",
    "Switch density to inspect the spacing between data rows.",
  ],
  "aspect-ratio": [
    "Resize the viewport to inspect the preserved 16:9 frame.",
    "Content keeps its proportions in either density.",
  ],
  carousel: [
    "Move through the three slides using Next and Previous.",
    "Inspect the announced position and disabled actions at the ends.",
  ],
  "scroll-area": [
    "Scroll through all 20 activities.",
    "Inspect the thin thumb as its position changes; keyboard scrolling works in the viewport.",
  ],
  resizable: [
    "Drag the separator to resize the two regions.",
    "Focus the separator and use arrow keys to resize; the visible grip remains separate from its hit area.",
  ],
};
