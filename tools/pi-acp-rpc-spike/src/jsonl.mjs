import { StringDecoder } from "node:string_decoder";

const DEFAULT_MAX_LINE_BYTES = 10_000_000;

/**
 * Attach an LF-only JSONL parser. U+2028 and U+2029 remain ordinary JSON text.
 */
export function attachJsonlReader(
  stream,
  onValue,
  onError,
  maxLineBytes = DEFAULT_MAX_LINE_BYTES,
) {
  const decoder = new StringDecoder("utf8");
  let buffer = "";

  const consume = (chunk) => {
    buffer += chunk;
    while (true) {
      const newline = buffer.indexOf("\n");
      if (newline === -1) break;
      let line = buffer.slice(0, newline);
      buffer = buffer.slice(newline + 1);
      if (line.endsWith("\r")) line = line.slice(0, -1);
      parseLine(line);
    }
    if (Buffer.byteLength(buffer, "utf8") > maxLineBytes) {
      onError(new Error(`JSONL line exceeds ${maxLineBytes} bytes`));
      buffer = "";
    }
  };

  const parseLine = (line) => {
    if (line.length === 0) return;
    if (Buffer.byteLength(line, "utf8") > maxLineBytes) {
      onError(new Error(`JSONL line exceeds ${maxLineBytes} bytes`));
      return;
    }
    try {
      onValue(JSON.parse(line));
    } catch (error) {
      onError(new Error(`invalid JSONL frame: ${error.message}`));
    }
  };

  stream.on("data", (chunk) => {
    consume(typeof chunk === "string" ? chunk : decoder.write(chunk));
  });
  stream.on("end", () => {
    consume(decoder.end());
    if (buffer.length > 0)
      parseLine(buffer.endsWith("\r") ? buffer.slice(0, -1) : buffer);
  });
}

export function writeJsonl(stream, value) {
  stream.write(`${JSON.stringify(value)}\n`);
}
