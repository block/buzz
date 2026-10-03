import type { Terminal } from "@earendil-works/pi-tui";
import xterm from "@xterm/headless";

export class TestTerminal implements Terminal {
  columns = 100;
  rows = 32;
  kittyProtocolActive = false;
  screen = new xterm.Terminal({ cols: 100, rows: 32, allowProposedApi: true });
  input: (data: string) => void = () => {};
  resize: () => void = () => {};
  start(input: (data: string) => void, resize: () => void): void {
    this.input = input;
    this.resize = resize;
  }
  stop(): void {}
  async drainInput(): Promise<void> {}
  write(data: string): void {
    this.screen.write(data);
  }
  moveBy(): void {}
  hideCursor(): void {}
  showCursor(): void {}
  clearLine(): void {}
  clearFromCursor(): void {}
  clearScreen(): void {}
  setTitle(): void {}
  setProgress(): void {}
  async frame(): Promise<string> {
    await new Promise((resolve) => setTimeout(resolve, 40));
    await new Promise<void>((resolve) => this.screen.write("", resolve));
    const buffer = this.screen.buffer.active;
    return Array.from(
      { length: this.rows },
      (_, row) =>
        buffer.getLine(row + buffer.viewportY)?.translateToString(true) ?? "",
    ).join("\n");
  }
}
