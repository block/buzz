import { useState } from "react";
import {
  Accordion,
  Breadcrumb,
  Button,
  Collapsible,
  Menu,
  Menubar,
  NavigationMenu,
  Pagination,
  Sidebar,
  Tabs,
  Toggle,
  ToggleGroup,
  Toolbar,
} from "@buzz/ui";
import { ChevronDown } from "lucide-react";
export function AccordionDemo() {
  return (
    <Accordion.Root>
      {[
        [
          "tokens",
          "One shared vocabulary",
          "Every component uses the same roles for color, type, and geometry.",
        ],
        [
          "behavior",
          "Accessible by construction",
          "Public primitives handle focus, keyboard navigation, and dismissal.",
        ],
      ].map(([id, title, content]) => (
        <Accordion.Item key={id} value={id}>
          <Accordion.Header>
            <Accordion.Trigger>
              {title}
              <ChevronDown aria-hidden="true" />
            </Accordion.Trigger>
          </Accordion.Header>
          <Accordion.Panel>{content}</Accordion.Panel>
        </Accordion.Item>
      ))}
    </Accordion.Root>
  );
}
export function CollapsibleDemo() {
  return (
    <Collapsible.Root>
      <Collapsible.Trigger render={<Button variant="outline" />}>
        Show implementation notes
      </Collapsible.Trigger>
      <Collapsible.Panel>
        <p className="text-body text-secondary mt-4">
          Compose components before adding a new abstraction. Keep product
          behavior in the feature that owns it.
        </p>
      </Collapsible.Panel>
    </Collapsible.Root>
  );
}
export function TabsDemo() {
  return (
    <Tabs.Root defaultValue="overview">
      <Tabs.List aria-label="Project views">
        <Tabs.Tab value="overview">Overview</Tabs.Tab>
        <Tabs.Tab value="activity">Activity</Tabs.Tab>
        <Tabs.Tab value="files">Files</Tabs.Tab>
      </Tabs.List>
      <Tabs.Panel value="overview">
        People and agents building together.
      </Tabs.Panel>
      <Tabs.Panel value="activity">Three tasks completed today.</Tabs.Panel>
      <Tabs.Panel value="files">Your project files live here.</Tabs.Panel>
    </Tabs.Root>
  );
}
export function ToggleDemo() {
  return <Toggle.Toggle aria-label="Pin message">Pin message</Toggle.Toggle>;
}
export function ToggleGroupDemo() {
  return (
    <ToggleGroup.ToggleGroup defaultValue={["day"]} aria-label="Timeline scale">
      {["day", "week", "month"].map((value) => (
        <Toggle.Toggle key={value} value={value} aria-label={`Show ${value}`}>
          {value}
        </Toggle.Toggle>
      ))}
    </ToggleGroup.ToggleGroup>
  );
}
export function ToolbarDemo() {
  const [action, setAction] = useState("No action selected");
  return (
    <div className="bui-stack">
      <Toolbar.Root aria-label="Editor actions">
        <Toolbar.Button onClick={() => setAction("Bold selected")}>
          Bold
        </Toolbar.Button>
        <Toolbar.Button onClick={() => setAction("Italic selected")}>
          Italic
        </Toolbar.Button>
        <Toolbar.Button onClick={() => setAction("Link selected")}>
          Link
        </Toolbar.Button>
      </Toolbar.Root>
      <p className="text-caption text-secondary" role="status">
        {action}
      </p>
    </div>
  );
}
export function MenubarDemo() {
  const [action, setAction] = useState("Choose a menu action");
  return (
    <div className="bui-stack">
      <Menubar.Root aria-label="Project menu">
        {["Project", "View"].map((name) => (
          <Menu.Root key={name}>
            <Menu.Trigger render={<Button variant="ghost" />}>
              {name}
            </Menu.Trigger>
            <Menu.Portal>
              <Menu.Positioner>
                <Menu.Popup>
                  <Menu.Item
                    onClick={() => setAction(`${name} settings selected`)}
                  >
                    Settings
                  </Menu.Item>
                  <Menu.Item
                    onClick={() => setAction(`${name} details selected`)}
                  >
                    Details
                  </Menu.Item>
                </Menu.Popup>
              </Menu.Positioner>
            </Menu.Portal>
          </Menu.Root>
        ))}
      </Menubar.Root>
      <p className="text-caption text-secondary" role="status">
        {action}
      </p>
    </div>
  );
}
export function NavigationDemo() {
  return (
    <NavigationMenu.Root>
      <NavigationMenu.List>
        <NavigationMenu.Item>
          <NavigationMenu.Link href="/design">Overview</NavigationMenu.Link>
        </NavigationMenu.Item>
        <NavigationMenu.Item>
          <NavigationMenu.Trigger>Foundations</NavigationMenu.Trigger>
          <NavigationMenu.Content>
            <NavigationMenu.Link href="/design/color">
              Color roles
            </NavigationMenu.Link>
            <NavigationMenu.Link href="/design/typography">
              Typography
            </NavigationMenu.Link>
          </NavigationMenu.Content>
        </NavigationMenu.Item>
      </NavigationMenu.List>
      <NavigationMenu.Portal>
        <NavigationMenu.Positioner sideOffset={8}>
          <NavigationMenu.Popup>
            <NavigationMenu.Viewport />
          </NavigationMenu.Popup>
        </NavigationMenu.Positioner>
      </NavigationMenu.Portal>
    </NavigationMenu.Root>
  );
}
export function BreadcrumbDemo() {
  return (
    <Breadcrumb>
      <li>
        <a href="/design">Design system</a>
      </li>
      <li>
        <a href="/design/components">Components</a>
      </li>
      <li aria-current="page">Breadcrumb</li>
    </Breadcrumb>
  );
}
export function SidebarDemo() {
  return (
    <Sidebar aria-label="Example project navigation">
      <a href="/design">Overview</a>
      <a href="/design/components" aria-current="page">
        Components
      </a>
      <a href="/design/color">Color</a>
    </Sidebar>
  );
}
export function PaginationDemo() {
  const [page, setPage] = useState(1);
  return (
    <div className="bui-stack">
      <Pagination>
        <Button
          variant="outline"
          size="sm"
          disabled={page === 1}
          onClick={() => setPage(page - 1)}
        >
          Previous
        </Button>
        {[1, 2, 3].map((number) => (
          <Button
            key={number}
            size="sm"
            variant={page === number ? "primary" : "ghost"}
            aria-label={`Page ${number}`}
            aria-current={page === number ? "page" : undefined}
            onClick={() => setPage(number)}
          >
            {number}
          </Button>
        ))}
        <Button
          variant="outline"
          size="sm"
          disabled={page === 3}
          onClick={() => setPage(page + 1)}
        >
          Next
        </Button>
      </Pagination>
      <p className="text-caption text-secondary" role="status">
        Showing page {page} of 3
      </p>
    </div>
  );
}
