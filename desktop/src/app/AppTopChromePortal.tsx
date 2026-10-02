import * as React from "react";
import { createPortal } from "react-dom";

const APP_TOP_CHROME_CONTENT_ID = "app-top-chrome-content";

export function AppTopChromePortal({
  children,
}: {
  children: React.ReactNode;
}) {
  const [target, setTarget] = React.useState<HTMLElement | null>(null);

  React.useEffect(() => {
    setTarget(document.getElementById(APP_TOP_CHROME_CONTENT_ID));
  }, []);

  return target ? createPortal(children, target) : null;
}
