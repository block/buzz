import { createContext } from "react";

/** Opt a conversation surface into bubbles, using its signed-in viewer identity. */
export const MessageBubbleContext = createContext<string | null>(null);
