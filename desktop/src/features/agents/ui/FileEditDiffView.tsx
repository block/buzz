import type {
  FileEditDiff,
  FileEditDiffLine,
} from "./agentSessionFileEditDiff";
import { FileContentBlock, type FileContentLine } from "./FileContentBlock";

export function hasFileEditLineDiff(diff: FileEditDiff) {
  return diff.lines.some(
    (line) => line.kind === "add" || line.kind === "remove",
  );
}

export function FileEditDiffBlock({ diff }: { diff: FileEditDiff }) {
  return (
    <FileContentBlock
      lines={diff.lines
        .filter((line) => line.kind !== "meta")
        .map(toFileContentLine)}
      path={diff.path}
    />
  );
}

function toFileContentLine(line: FileEditDiffLine): FileContentLine {
  return {
    kind: line.kind,
    text: line.text,
  };
}
