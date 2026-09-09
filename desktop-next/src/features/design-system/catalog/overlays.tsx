import { useState } from "react";
import {
  AlertDialog,
  Button,
  ContextMenu,
  Dialog,
  Drawer,
  Menu,
  Popover,
  PreviewCard,
  Select,
  Sheet,
  Toast,
  Tooltip,
} from "@buzz/ui";
import { Check, ChevronDown, X } from "lucide-react";

export function DialogDemo() {
  return (
    <Dialog.Root>
      <Dialog.Trigger render={<Button variant="outline" />}>
        Edit project
      </Dialog.Trigger>
      <Dialog.Portal>
        <Dialog.Backdrop />
        <Dialog.Popup>
          <Dialog.Title>A place to build together</Dialog.Title>
          <Dialog.Description>
            Project settings stay with your community. This preview does not
            save changes.
          </Dialog.Description>
          <Dialog.Close render={<Button />}>Done</Dialog.Close>
        </Dialog.Popup>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
export function AlertDialogDemo() {
  const [deleted, setDeleted] = useState(false);
  return (
    <div className="bui-stack">
      <AlertDialog.Root>
        <AlertDialog.Trigger render={<Button variant="danger" />}>
          Delete draft
        </AlertDialog.Trigger>
        <AlertDialog.Portal>
          <AlertDialog.Backdrop />
          <AlertDialog.Popup>
            <AlertDialog.Title>Delete this draft?</AlertDialog.Title>
            <AlertDialog.Description>
              This removes the example draft from this preview. You can restore
              it below.
            </AlertDialog.Description>
            <div className="bui-inline">
              <AlertDialog.Close render={<Button variant="outline" />}>
                Keep draft
              </AlertDialog.Close>
              <AlertDialog.Close
                render={<Button variant="danger" />}
                onClick={() => setDeleted(true)}
              >
                Delete example draft
              </AlertDialog.Close>
            </div>
          </AlertDialog.Popup>
        </AlertDialog.Portal>
      </AlertDialog.Root>
      {deleted && (
        <Button variant="link" onClick={() => setDeleted(false)}>
          Draft deleted · Restore
        </Button>
      )}
    </div>
  );
}
export function SheetDemo() {
  return (
    <Sheet.Root>
      <Sheet.Trigger render={<Button variant="outline" />}>
        Project details
      </Sheet.Trigger>
      <Sheet.Portal>
        <Sheet.Backdrop />
        <Sheet.Popup>
          <Sheet.Title>Design system</Sheet.Title>
          <Sheet.Description>
            A shared language for people and agents building together.
          </Sheet.Description>
          <p className="text-body">
            Inter, Lucide, semantic roles, and open-source behavior.
          </p>
          <Sheet.Close render={<Button />}>Close details</Sheet.Close>
        </Sheet.Popup>
      </Sheet.Portal>
    </Sheet.Root>
  );
}
export function DrawerDemo() {
  return (
    <Drawer.Root>
      <Drawer.Trigger render={<Button variant="outline" />}>
        Open drawer
      </Drawer.Trigger>
      <Drawer.Portal>
        <Drawer.Backdrop />
        <Drawer.Viewport>
          <Drawer.Popup>
            <Drawer.Content>
              <div className="bui-stack">
                <Drawer.Title>Take the next step</Drawer.Title>
                <Drawer.Description>
                  A bottom surface for focused actions.
                </Drawer.Description>
                <Drawer.Close render={<Button />}>
                  Continue building
                </Drawer.Close>
              </div>
            </Drawer.Content>
          </Drawer.Popup>
        </Drawer.Viewport>
      </Drawer.Portal>
    </Drawer.Root>
  );
}
export function PopoverDemo() {
  return (
    <Popover.Root>
      <Popover.Trigger render={<Button variant="outline" />}>
        Share options
      </Popover.Trigger>
      <Popover.Portal>
        <Popover.Positioner sideOffset={8}>
          <Popover.Popup>
            <Popover.Title>Share your progress</Popover.Title>
            <Popover.Description>
              People in your project can already see this work.
            </Popover.Description>
            <Popover.Close render={<Button variant="secondary" />}>
              Got it
            </Popover.Close>
          </Popover.Popup>
        </Popover.Positioner>
      </Popover.Portal>
    </Popover.Root>
  );
}
export function TooltipDemo() {
  return (
    <Tooltip.Provider>
      <Tooltip.Root>
        <Tooltip.Trigger render={<Button variant="outline" />}>
          Hover or focus me
        </Tooltip.Trigger>
        <Tooltip.Portal>
          <Tooltip.Positioner sideOffset={8}>
            <Tooltip.Popup>Helpful context without a detour.</Tooltip.Popup>
          </Tooltip.Positioner>
        </Tooltip.Portal>
      </Tooltip.Root>
    </Tooltip.Provider>
  );
}
export function PreviewCardDemo() {
  return (
    <PreviewCard.Root>
      <PreviewCard.Trigger
        href="#preview-card"
        className="text-accent underline"
      >
        Design systems team
      </PreviewCard.Trigger>
      <PreviewCard.Portal>
        <PreviewCard.Positioner sideOffset={8}>
          <PreviewCard.Popup>
            <p className="text-heading">Building in the open</p>
            <p className="text-body text-secondary">
              A small team creating tools for everyone.
            </p>
          </PreviewCard.Popup>
        </PreviewCard.Positioner>
      </PreviewCard.Portal>
    </PreviewCard.Root>
  );
}
export function MenuDemo() {
  const [selected, setSelected] = useState("Choose an action");
  return (
    <div className="bui-stack">
      <Menu.Root>
        <Menu.Trigger render={<Button variant="outline" />}>
          Project actions <ChevronDown aria-hidden="true" />
        </Menu.Trigger>
        <Menu.Portal>
          <Menu.Positioner sideOffset={8}>
            <Menu.Popup>
              {["Rename project", "Copy link", "Archive project"].map(
                (label) => (
                  <Menu.Item key={label} onClick={() => setSelected(label)}>
                    {label}
                  </Menu.Item>
                ),
              )}
            </Menu.Popup>
          </Menu.Positioner>
        </Menu.Portal>
      </Menu.Root>
      <p role="status" className="text-caption text-secondary">
        {selected}
      </p>
    </div>
  );
}
export function ContextMenuDemo() {
  const [action, setAction] = useState("Right-click or use Shift + F10");
  return (
    <ContextMenu.Root>
      <ContextMenu.Trigger tabIndex={0}>{action}</ContextMenu.Trigger>
      <ContextMenu.Portal>
        <ContextMenu.Positioner>
          <ContextMenu.Popup>
            <ContextMenu.Item onClick={() => setAction("Copied example link")}>
              Copy link
            </ContextMenu.Item>
            <ContextMenu.Item onClick={() => setAction("Marked as unread")}>
              Mark as unread
            </ContextMenu.Item>
          </ContextMenu.Popup>
        </ContextMenu.Positioner>
      </ContextMenu.Portal>
    </ContextMenu.Root>
  );
}
export function SelectDemo() {
  return (
    <Select.Root
      defaultValue="daily"
      items={{ daily: "Daily digest", weekly: "Weekly digest", never: "Never" }}
    >
      <Select.Trigger aria-label="Digest frequency">
        <Select.Value />
        <Select.Icon>
          <ChevronDown aria-hidden="true" className="bui-icon" />
        </Select.Icon>
      </Select.Trigger>
      <Select.Portal>
        <Select.Positioner sideOffset={8}>
          <Select.Popup>
            <Select.List>
              {[
                ["daily", "Daily digest"],
                ["weekly", "Weekly digest"],
                ["never", "Never"],
              ].map(([value, label]) => (
                <Select.Item key={value} value={value}>
                  <Select.ItemText>{label}</Select.ItemText>
                  <Select.ItemIndicator>
                    <Check aria-hidden="true" />
                  </Select.ItemIndicator>
                </Select.Item>
              ))}
            </Select.List>
          </Select.Popup>
        </Select.Positioner>
      </Select.Portal>
    </Select.Root>
  );
}
function ToastContent() {
  const manager = Toast.useToastManager();
  return (
    <>
      <Button
        variant="outline"
        onClick={() =>
          manager.add({
            title: "Changes saved",
            description: "Your project is up to date.",
          })
        }
      >
        Show notification
      </Button>
      <Toast.Portal>
        <Toast.Viewport>
          {manager.toasts.map((toast) => (
            <Toast.Root key={toast.id} toast={toast}>
              <Toast.Content>
                <Toast.Title />
                <Toast.Description />
              </Toast.Content>
              <Toast.Close aria-label="Dismiss notification">
                <X aria-hidden="true" className="bui-icon" />
              </Toast.Close>
            </Toast.Root>
          ))}
        </Toast.Viewport>
      </Toast.Portal>
    </>
  );
}
export function ToastDemo() {
  return (
    <Toast.Provider limit={3}>
      <ToastContent />
    </Toast.Provider>
  );
}
