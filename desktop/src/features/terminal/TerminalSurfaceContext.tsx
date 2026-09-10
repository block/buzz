import * as React from "react";

/** Lets an embedded conversation surface place the shell-owned terminal. */
export const TerminalSurfaceContext =
  React.createContext<React.ReactNode>(null);
