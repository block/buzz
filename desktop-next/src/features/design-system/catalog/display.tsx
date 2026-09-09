import { Link } from "@tanstack/react-router";
import {
  Alert,
  AspectRatio,
  Avatar,
  Badge,
  BarChart,
  Button,
  Card,
  Carousel,
  EmptyState,
  Meter,
  Progress,
  Resizable,
  ScrollArea,
  Separator,
  Skeleton,
  Spinner,
  Table,
} from "@buzz/ui";
import { Check, Layers, Sparkles } from "lucide-react";
export function CardDemo() {
  return (
    <Card>
      <div className="bui-inline">
        <Layers aria-hidden="true" />
        <Badge>Project</Badge>
      </div>
      <h3 className="text-heading">Build something together</h3>
      <p className="text-body text-secondary">
        A quiet container for a focused group of ideas.
      </p>
      <Button
        nativeButton={false}
        role="link"
        render={<Link to="/design/compositions" />}
      >
        Explore compositions
      </Button>
    </Card>
  );
}
export function BadgeDemo() {
  return (
    <div className="bui-inline">
      <Badge>Draft</Badge>
      <Badge tone="accent">In progress</Badge>
      <Badge tone="success">
        <Check aria-hidden="true" className="bui-icon" />
        Complete
      </Badge>
      <Badge tone="warning">Needs attention</Badge>
      <Badge tone="danger">Failed</Badge>
      <Badge tone="info">Information</Badge>
    </div>
  );
}
export function AlertDemo() {
  return (
    <div className="bui-stack">
      <Alert tone="info">Your changes are visible only in this preview.</Alert>
      <Alert tone="success">All tasks completed.</Alert>
      <Alert tone="warning">One task needs your attention.</Alert>
      <Alert tone="danger">
        Unable to connect. Check your connection and try again.
      </Alert>
    </div>
  );
}
export function AvatarDemo() {
  return (
    <div className="bui-inline">
      {["AJ", "TB", "DS"].map((name) => (
        <Avatar.Root key={name} role="img" aria-label={name}>
          <Avatar.Fallback>{name}</Avatar.Fallback>
        </Avatar.Root>
      ))}
      <Avatar.Root role="img" aria-label="Agent">
        <Avatar.Fallback>
          <Sparkles aria-hidden="true" className="bui-icon" />
        </Avatar.Fallback>
      </Avatar.Root>
    </div>
  );
}
export function ProgressDemo() {
  return (
    <Progress.Root value={65}>
      <div className="bui-inline">
        <Progress.Label>Uploading files</Progress.Label>
        <Progress.Value />
      </div>
      <Progress.Track>
        <Progress.Indicator />
      </Progress.Track>
    </Progress.Root>
  );
}
export function MeterDemo() {
  return (
    <Meter.Root value={42}>
      <div className="bui-inline">
        <Meter.Label>Storage used</Meter.Label>
        <Meter.Value />
      </div>
      <Meter.Track>
        <Meter.Indicator />
      </Meter.Track>
    </Meter.Root>
  );
}
export function SkeletonDemo() {
  return (
    <section
      aria-label="Loading project"
      aria-busy="true"
      className="bui-stack"
    >
      <Skeleton />
      <Skeleton style={{ width: "70%" }} />
      <span className="bui-sr-only">Loading project</span>
    </section>
  );
}
export function SpinnerDemo() {
  return <Spinner label="Connecting to your project" />;
}
export function EmptyDemo() {
  return (
    <EmptyState
      title="Room for your next idea"
      description="Create a project to bring people, agents, and their work together."
      action={
        <Button
          nativeButton={false}
          role="link"
          render={<Link to="/design/compositions" />}
        >
          See an example project
        </Button>
      }
    />
  );
}
export function SeparatorDemo() {
  return (
    <div className="text-body">
      Project overview
      <Separator />
      Recent activity
    </div>
  );
}
export function TableDemo() {
  return (
    <Table>
      <caption className="text-label">Recent tasks</caption>
      <thead>
        <tr>
          <th scope="col">Task</th>
          <th scope="col">Owner</th>
          <th scope="col">Status</th>
        </tr>
      </thead>
      <tbody>
        {[
          ["Review tokens", "Alex", "Complete"],
          ["Build components", "Sam", "In progress"],
          ["Write guidance", "Riley", "Draft"],
        ].map(([task, owner, status]) => (
          <tr key={task}>
            <th scope="row">{task}</th>
            <td>{owner}</td>
            <td>
              <Badge tone={status === "Complete" ? "success" : "neutral"}>
                {status}
              </Badge>
            </td>
          </tr>
        ))}
      </tbody>
    </Table>
  );
}
export function ChartDemo() {
  return (
    <BarChart
      title="Tasks completed this week"
      data={[
        { label: "Monday", value: 12 },
        { label: "Tuesday", value: 19 },
        { label: "Wednesday", value: 8 },
        { label: "Thursday", value: 24 },
        { label: "Friday", value: 16 },
      ]}
    />
  );
}
export function AspectRatioDemo() {
  return (
    <AspectRatio
      className="rounded-container bg-inset flex items-center justify-center text-heading"
      ratio={16 / 9}
    >
      16 : 9
    </AspectRatio>
  );
}
export function CarouselDemo() {
  return (
    <Carousel
      label="Design principles"
      slides={[
        "Start with roles",
        "Compose primitives",
        "Build in the open",
      ].map((title, index) => (
        <div key={title} className="rounded-container bg-inset p-8">
          <p className="text-caption text-secondary">Principle {index + 1}</p>
          <h3 className="text-heading mt-2">{title}</h3>
        </div>
      ))}
    />
  );
}
export function ScrollAreaDemo() {
  return (
    <ScrollArea.Root>
      <ScrollArea.Viewport>
        <div>
          <ol className="bui-stack text-body">
            {Array.from({ length: 20 }, (_, i) => `Activity ${i + 1}`).map(
              (label) => (
                <li key={label}>{label}</li>
              ),
            )}
          </ol>
        </div>
      </ScrollArea.Viewport>
      <ScrollArea.Scrollbar>
        <ScrollArea.Thumb />
      </ScrollArea.Scrollbar>
    </ScrollArea.Root>
  );
}
export function ResizableDemo() {
  return (
    <Resizable.Group orientation="horizontal" style={{ height: "12rem" }}>
      <Resizable.Panel defaultSize="40%" minSize="20%">
        <div className="p-4 text-body bg-inset h-full rounded-control">
          Project list
        </div>
      </Resizable.Panel>
      <Resizable.Handle aria-label="Resize project list" />
      <Resizable.Panel minSize="20%">
        <div className="p-4 text-body bg-inset h-full rounded-control">
          Conversation
        </div>
      </Resizable.Panel>
    </Resizable.Group>
  );
}
