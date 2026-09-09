import { useId, useState } from "react";
import {
  Avatar,
  Badge,
  BarChart,
  Button,
  Card,
  Field,
  GeneratedResponse,
  Input,
  Switch,
  Tabs,
  type GeneratedView,
} from "@buzz/ui";
import { ArrowUpRight, Check, Sparkles } from "lucide-react";
import { PageHeader, Section } from "./primitives";
const response: GeneratedView = {
  version: 1,
  title: "Design system update",
  blocks: [
    {
      id: "summary",
      type: "text",
      text: "The shared foundation is ready. Here is the current state of the work.",
    },
    {
      id: "tasks",
      type: "tasks",
      items: [
        {
          id: "tokens",
          label: "Open-source tokens and typography",
          status: "complete",
        },
        {
          id: "components",
          label: "Accessible component primitives",
          status: "complete",
        },
        {
          id: "catalog",
          label: "Interactive catalog and examples",
          status: "running",
        },
      ],
    },
    {
      id: "notice",
      type: "notice",
      tone: "info",
      text: "Draft changes stay available for iteration before review.",
    },
    {
      id: "actions",
      type: "actions",
      items: [
        { id: "review", label: "Inspect changes" },
        { id: "details", label: "View task details" },
      ],
    },
  ],
};
const report: GeneratedView = {
  version: 1,
  title: "Weekly project report",
  blocks: [
    {
      id: "metric",
      type: "metric",
      label: "Tasks completed",
      value: "24",
      detail: "Across four active projects",
    },
    {
      id: "table",
      type: "table",
      columns: ["Project", "Completed", "In progress"],
      rows: [
        ["Design system", "12", "3"],
        ["Desktop", "8", "2"],
        ["Relay", "4", "1"],
      ],
    },
    {
      id: "actions",
      type: "actions",
      items: [{ id: "report", label: "Open full report" }],
    },
  ],
};
export function CompositionsPage() {
  const notificationId = useId();
  const [streaming, setStreaming] = useState(false);
  const [invalid, setInvalid] = useState(false);
  const [action, setAction] = useState(
    "Actions in these examples update this preview only.",
  );
  const [saved, setSaved] = useState(false);
  return (
    <>
      <PageHeader
        title="Built from the same pieces"
        intro="Product screens and generated responses share one visual language. These examples compose the library directly, without a second set of styles or a desktop bridge."
      />
      <Section
        title="Project workspace"
        description="Identity, progress, and focused actions. Color earns its place through status."
      >
        <div className="grid gap-6 md:grid-cols-2">
          <Card>
            <div className="bui-inline">
              <Badge>Project</Badge>
              <Badge tone="success">
                <Check aria-hidden="true" className="bui-icon" />
                On track
              </Badge>
            </div>
            <h3 className="text-title">Building in the open</h3>
            <p className="text-body text-secondary">
              A shared place for people, agents, and ideas to become working
              software.
            </p>
            <div className="bui-inline">
              {["AJ", "TB", "DS"].map((name) => (
                <Avatar.Root key={name} role="img" aria-label={name}>
                  <Avatar.Fallback>{name}</Avatar.Fallback>
                </Avatar.Root>
              ))}
              <span className="text-caption text-secondary">
                3 people · 2 agents
              </span>
            </div>
            <Button onClick={() => setAction("Project opened in this preview")}>
              Open project <ArrowUpRight aria-hidden="true" />
            </Button>
          </Card>
          <Card>
            <BarChart
              title="Tasks completed"
              data={[
                { label: "Mon", value: 9 },
                { label: "Tue", value: 16 },
                { label: "Wed", value: 12 },
                { label: "Thu", value: 24 },
                { label: "Fri", value: 18 },
              ]}
            />
            <p className="text-caption text-secondary">
              Shared work becomes visible progress.
            </p>
          </Card>
        </div>
      </Section>
      <Section
        title="Settings"
        description="Labels, help, validation, and action emphasis form one consistent flow."
      >
        <Card>
          <Tabs.Root defaultValue="profile">
            <Tabs.List aria-label="Settings section">
              <Tabs.Tab value="profile">Profile</Tabs.Tab>
              <Tabs.Tab value="notifications">Notifications</Tabs.Tab>
            </Tabs.List>
            <Tabs.Panel value="profile">
              <form
                className="bui-stack"
                onSubmit={(event) => {
                  event.preventDefault();
                  setSaved(true);
                }}
              >
                <Field.Root>
                  <Field.Label>Display name</Field.Label>
                  <Input defaultValue="Alex Jordan" required />
                  <Field.Description>
                    The name people see in your projects.
                  </Field.Description>
                </Field.Root>
                <Field.Root>
                  <Field.Label>About you</Field.Label>
                  <Input placeholder="What do you like to build?" />
                </Field.Root>
                <div className="bui-inline">
                  <Button type="submit">Save profile</Button>
                  {saved && (
                    <span role="status" className="text-caption text-success">
                      Profile saved in this preview.
                    </span>
                  )}
                </div>
              </form>
            </Tabs.Panel>
            <Tabs.Panel value="notifications">
              <label htmlFor={notificationId} className="bui-inline text-body">
                <Switch.Root id={notificationId} defaultChecked>
                  <Switch.Thumb />
                </Switch.Root>
                Notify me when an agent needs attention
              </label>
            </Tabs.Panel>
          </Tabs.Root>
        </Card>
      </Section>
      <Section
        title="Generated responses"
        description="An agent supplies a versioned data snapshot. The host validates it and maps known actions to explicit callbacks."
      >
        <div className="bui-inline mb-6">
          <Button
            variant="outline"
            aria-pressed={streaming}
            onClick={() => setStreaming(!streaming)}
          >
            {streaming ? "Finish streaming" : "Preview streaming"}
          </Button>
          <Button
            variant="outline"
            aria-pressed={invalid}
            onClick={() => setInvalid(!invalid)}
          >
            {invalid ? "Restore valid response" : "Preview invalid response"}
          </Button>
        </div>
        <div className="grid gap-6 md:grid-cols-2">
          <GeneratedResponse
            response={invalid ? { version: 99 } : response}
            state={streaming ? "streaming" : "complete"}
            actions={{
              review: () => setAction("Inspect changes requested"),
              details: () => setAction("Task details requested"),
            }}
            onRetry={() => setInvalid(false)}
          />
          <GeneratedResponse
            response={report}
            actions={{ report: () => setAction("Full report requested") }}
          />
        </div>
        <div className="bui-inline mt-6 text-caption text-secondary">
          <Sparkles aria-hidden="true" />
          <p role="status">{action}</p>
        </div>
        <details className="catalog-source mt-6">
          <summary className="text-caption text-secondary">
            View the response data
          </summary>
          <pre className="text-code">
            <code>{JSON.stringify(response, null, 2)}</code>
          </pre>
        </details>
      </Section>
    </>
  );
}
