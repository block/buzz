import { getCanvas, setCanvas } from "@/shared/api/tauri";

export const WELCOME_CANVAS_CONTENT = `# Welcome to Buzz

This private channel is your home base for getting oriented. Fizz, Honey, and Bumble can help you learn the app, troubleshoot setup, and work through something you are building.

## Work with your agents

- Mention an agent when you want its help.
- Bring multiple agents into the same conversation when you want different perspectives.
- Keep decisions, progress, and results in the channel so everyone shares the same context.

## Try something

Bring the team something you are building, or give them a quick challenge to see how they work together.

## Get help

Ask the team a question here, or read the [Buzz user guide](https://github.com/block/buzz#readme).
`;

/** Seed the Welcome canvas without overwriting anything the user has written. */
export async function ensureWelcomeCanvas(channelId: string) {
  const existing = await getCanvas(channelId);
  if (existing.updatedAt !== null || existing.author !== null) {
    return false;
  }

  await setCanvas({ channelId, content: WELCOME_CANVAS_CONTENT });
  return true;
}
