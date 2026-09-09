import { useId, useState } from "react";
import {
  ArrowRight,
  Check,
  ChevronDown,
  Minus,
  Plus,
  Search,
} from "lucide-react";
import {
  Autocomplete,
  Button,
  Calendar,
  Checkbox,
  CheckboxGroup,
  Combobox,
  Command,
  Field,
  Fieldset,
  Form,
  IconButton,
  Input,
  InputGroup,
  NumberField,
  OTPField,
  Radio,
  RadioGroup,
  Slider,
  Switch,
  Textarea,
} from "@buzz/ui";

export function ButtonsDemo() {
  const [count, setCount] = useState(0);
  return (
    <div className="bui-stack">
      <div className="bui-inline">
        <Button onClick={() => setCount(count + 1)}>
          Continue <ArrowRight aria-hidden="true" />
        </Button>
        <Button variant="secondary">Secondary</Button>
        <Button variant="outline">Outline</Button>
        <Button variant="ghost">Ghost</Button>
        <Button variant="danger">Delete</Button>
        <Button variant="link">Learn more</Button>
      </div>
      <div className="bui-inline">
        <Button size="sm">Small</Button>
        <Button size="lg">Large</Button>
        <Button disabled>Unavailable</Button>
        <Button loading>Saving</Button>
        <IconButton
          aria-label="Add item"
          variant="outline"
          onClick={() => setCount(count + 1)}
        >
          <Plus aria-hidden="true" />
        </IconButton>
      </div>
      <p className="text-caption text-secondary" role="status">
        {count ? `${count} actions taken` : "Try Continue or Add item"}
      </p>
    </div>
  );
}
export function InputDemo() {
  return (
    <Field.Root>
      <Field.Label>Project name</Field.Label>
      <Input placeholder="A place to build" />
      <Field.Description>Visible to everyone in the project.</Field.Description>
    </Field.Root>
  );
}
export function TextareaDemo() {
  return (
    <Field.Root>
      <Field.Label>Project description</Field.Label>
      <Textarea placeholder="What are you building?" />
    </Field.Root>
  );
}
export function InputGroupDemo() {
  const id = useId();
  return (
    <div className="bui-stack">
      <label className="bui-label" htmlFor={id}>
        Search messages
      </label>
      <InputGroup>
        <Search aria-hidden="true" className="bui-icon" />
        <Input id={id} placeholder="Search this project" />
        <kbd className="text-code">⌘ K</kbd>
      </InputGroup>
    </div>
  );
}
export function FormDemo() {
  const [saved, setSaved] = useState(false);
  return (
    <Form.Root
      onSubmit={(event) => {
        event.preventDefault();
        setSaved(true);
      }}
    >
      <div className="bui-stack">
        <Field.Root name="email">
          <Field.Label>Email</Field.Label>
          <Input type="email" required placeholder="you@example.org" />
          <Field.Description>Used for your invitation.</Field.Description>
          <Field.Error />
        </Field.Root>
        <Button type="submit">Invite member</Button>
        {saved && (
          <p role="status" className="text-caption text-success">
            Invitation prepared in this demo.
          </p>
        )}
      </div>
    </Form.Root>
  );
}
export function CheckboxDemo() {
  const id = useId();
  return (
    <label htmlFor={id} className="bui-inline text-body">
      <Checkbox.Root id={id} defaultChecked>
        <Checkbox.Indicator>
          <Check aria-hidden="true" />
        </Checkbox.Indicator>
      </Checkbox.Root>
      Notify me about replies
    </label>
  );
}
export function CheckboxGroupDemo() {
  const id = useId();
  return (
    <Fieldset.Root>
      <Fieldset.Legend>Notify me in</Fieldset.Legend>
      <CheckboxGroup.Root defaultValue={["mentions"]}>
        {["mentions", "threads", "projects"].map((item) => (
          <label
            key={item}
            htmlFor={`${id}-${item}`}
            className="bui-inline text-body"
          >
            <Checkbox.Root id={`${id}-${item}`} value={item}>
              <Checkbox.Indicator>
                <Check aria-hidden="true" />
              </Checkbox.Indicator>
            </Checkbox.Root>
            {item}
          </label>
        ))}
      </CheckboxGroup.Root>
    </Fieldset.Root>
  );
}
export function RadioDemo() {
  const id = useId();
  return (
    <Fieldset.Root>
      <Fieldset.Legend>Visibility</Fieldset.Legend>
      <RadioGroup.Root defaultValue="team">
        {["team", "public"].map((item) => (
          <label
            key={item}
            htmlFor={`${id}-${item}`}
            className="bui-inline text-body"
          >
            <Radio.Root id={`${id}-${item}`} value={item}>
              <Radio.Indicator />
            </Radio.Root>
            {item === "team" ? "Team only" : "Anyone with the link"}
          </label>
        ))}
      </RadioGroup.Root>
    </Fieldset.Root>
  );
}
export function SwitchDemo() {
  const id = useId();
  return (
    <label htmlFor={id} className="bui-inline text-body">
      <Switch.Root id={id} defaultChecked>
        <Switch.Thumb />
      </Switch.Root>
      Desktop notifications
    </label>
  );
}
export function SliderDemo() {
  return (
    <Slider.Root defaultValue={60}>
      <div className="bui-inline">
        <Slider.Label>Playback volume</Slider.Label>
        <Slider.Value />
      </div>
      <Slider.Control>
        <Slider.Track>
          <Slider.Indicator />
          <Slider.Thumb aria-label="Playback volume" />
        </Slider.Track>
      </Slider.Control>
    </Slider.Root>
  );
}
export function NumberDemo() {
  const id = useId();
  return (
    <NumberField.Root id={id} defaultValue={3} min={1} max={12}>
      <label htmlFor={id} className="bui-label">
        Parallel tasks
      </label>
      <NumberField.Group>
        <NumberField.Decrement aria-label="Fewer tasks">
          <Minus aria-hidden="true" />
        </NumberField.Decrement>
        <NumberField.Input />
        <NumberField.Increment aria-label="More tasks">
          <Plus aria-hidden="true" />
        </NumberField.Increment>
      </NumberField.Group>
    </NumberField.Root>
  );
}
export function OTPDemo() {
  return (
    <Field.Root>
      <Field.Label>Verification code</Field.Label>
      <OTPField.Root length={6}>
        {["First", "Second", "Third", "Fourth", "Fifth", "Sixth"].map(
          (position) => (
            <OTPField.Input key={position} aria-label={`${position} digit`} />
          ),
        )}
      </OTPField.Root>
      <Field.Description>Six digits from your device.</Field.Description>
    </Field.Root>
  );
}
const projects = ["Design system", "Relay", "Desktop", "Mobile"];
export function ComboboxDemo() {
  return (
    <Combobox.Root items={projects}>
      <Combobox.InputGroup>
        <Combobox.Input
          aria-label="Choose a project"
          placeholder="Find a project…"
        />
        <Combobox.Trigger aria-label="Show projects">
          <ChevronDown aria-hidden="true" />
        </Combobox.Trigger>
      </Combobox.InputGroup>
      <Combobox.Portal>
        <Combobox.Positioner sideOffset={8}>
          <Combobox.Popup>
            <Combobox.Empty>No projects found</Combobox.Empty>
            <Combobox.List>
              {(item: string) => (
                <Combobox.Item key={item} value={item}>
                  {item}
                  <Combobox.ItemIndicator>
                    <Check aria-hidden="true" />
                  </Combobox.ItemIndicator>
                </Combobox.Item>
              )}
            </Combobox.List>
          </Combobox.Popup>
        </Combobox.Positioner>
      </Combobox.Portal>
    </Combobox.Root>
  );
}
export function AutocompleteDemo() {
  return (
    <Autocomplete.Root items={projects}>
      <Autocomplete.Input
        aria-label="Project suggestion"
        placeholder="Type a project name"
      />
      <Autocomplete.Portal>
        <Autocomplete.Positioner sideOffset={8}>
          <Autocomplete.Popup>
            <Autocomplete.List>
              {(item: string) => (
                <Autocomplete.Item key={item} value={item}>
                  {item}
                </Autocomplete.Item>
              )}
            </Autocomplete.List>
          </Autocomplete.Popup>
        </Autocomplete.Positioner>
      </Autocomplete.Portal>
    </Autocomplete.Root>
  );
}
export function CommandDemo() {
  const [action, setAction] = useState("No command selected");
  return (
    <div className="bui-stack">
      <Command
        label="Find a command"
        items={[
          { id: "project", label: "Create project" },
          { id: "search", label: "Search messages" },
          { id: "settings", label: "Open settings" },
        ]}
        onAction={(id) => setAction(`Selected: ${id}`)}
      />
      <p role="status" className="text-caption text-secondary">
        {action}
      </p>
    </div>
  );
}
export function CalendarDemo() {
  const [date, setDate] = useState<Date | undefined>(new Date(2026, 8, 9));
  return (
    <Calendar
      mode="single"
      defaultMonth={new Date(2026, 8, 1)}
      selected={date}
      onSelect={setDate}
      footer={
        date ? `Selected ${date.toLocaleDateString("en-US")}` : "Choose a date"
      }
    />
  );
}
