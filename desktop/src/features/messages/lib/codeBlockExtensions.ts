import { Extension, InputRule } from "@tiptap/core";
import type { Editor } from "@tiptap/react";
import { TextSelection } from "@tiptap/pm/state";

const FENCE_AT_START = /^(?:```|~~~)([a-z+]*)$/;
const FENCE_AFTER_BREAK = /￼(?:```|~~~)([a-z+]*)$/;

/**
 * Detect ``` / ~~~ fence on Enter and create a code block instead of
 * submitting. Returns true/false for the keyboard shortcut handler,
 * or undefined when no fence was detected (caller should proceed).
 */
export function handleCodeFenceEnter(ed: Editor): boolean | undefined {
  if (ed.isActive("codeBlock")) return undefined;

  const { $cursor } = ed.state.selection as TextSelection;
  if (!$cursor) return undefined;

  const textBefore = $cursor.parent.textBetween(
    0,
    $cursor.parentOffset,
    null,
    "￼",
  );

  const startMatch = textBefore.match(FENCE_AT_START);
  if (startMatch) {
    const { tr, schema } = ed.state;
    const attrs = startMatch[1] ? { language: startMatch[1] } : {};
    tr.delete($cursor.start(), $cursor.pos);
    tr.setBlockType(
      tr.mapping.map($cursor.start()),
      tr.mapping.map($cursor.start()),
      schema.nodes.codeBlock,
      attrs,
    );
    ed.view.dispatch(tr);
    return true;
  }

  const m = textBefore.match(FENCE_AFTER_BREAK);
  if (!m) return undefined;

  const { tr, schema } = ed.state;
  const hardBreakDocPos =
    $cursor.start() + ($cursor.parentOffset - m[0].length);
  const afterParagraph = $cursor.after();
  tr.delete(hardBreakDocPos, $cursor.pos);
  const mapped = tr.mapping.map(afterParagraph);
  const attrs = m[1] ? { language: m[1] } : {};
  tr.insert(mapped, schema.nodes.codeBlock.create(attrs));
  tr.setSelection(TextSelection.near(tr.doc.resolve(mapped + 1)));
  ed.view.dispatch(tr);
  return true;
}

export function insertNewlineInCodeBlock(ed: Editor): boolean {
  return ed
    .chain()
    .focus()
    .command(({ tr, dispatch }) => {
      if (dispatch) {
        tr.replaceSelectionWith(ed.state.schema.text("\n"));
      }
      return true;
    })
    .run();
}

export function exitListIfEmptyLast(ed: Editor): boolean {
  if (!ed.isActive("listItem")) return false;
  const { $from } = ed.state.selection;

  let listItemDepth = -1;
  for (let depth = $from.depth; depth >= 1; depth--) {
    if ($from.node(depth).type.name === "listItem") {
      listItemDepth = depth;
      break;
    }
  }
  if (listItemDepth < 1) return false;

  const listItem = $from.node(listItemDepth);
  const isEmpty =
    listItem.childCount === 1 && listItem.firstChild?.textContent === "";
  if (!isEmpty) return false;

  const listDepth = listItemDepth - 1;
  const list = $from.node(listDepth);
  if ($from.index(listDepth) !== list.childCount - 1) return false;

  const { tr, schema } = ed.state;
  if (list.childCount === 1) {
    const listStart = $from.before(listDepth);
    tr.replaceWith(
      listStart,
      $from.after(listDepth),
      schema.nodes.paragraph.create(),
    );
    tr.setSelection(TextSelection.near(tr.doc.resolve(listStart + 1)));
  } else {
    tr.delete($from.before(listItemDepth), $from.after(listItemDepth));
    const listEnd = tr.mapping.map($from.after(listDepth));
    tr.insert(listEnd, schema.nodes.paragraph.create());
    tr.setSelection(TextSelection.near(tr.doc.resolve(listEnd + 1)));
  }
  ed.view.dispatch(tr);
  return true;
}

export function insertComposerSoftNewline(
  ed: Editor,
  { handleCodeFence }: { handleCodeFence: boolean },
): boolean {
  if (handleCodeFence) {
    const fenceResult = handleCodeFenceEnter(ed);
    if (fenceResult !== undefined) return fenceResult;
  }
  if (ed.isActive("codeBlock")) return insertNewlineInCodeBlock(ed);
  if (exitListIfEmptyLast(ed)) return true;
  if (ed.isActive("listItem")) {
    return ed.commands.splitListItem("listItem");
  }
  if (ed.isActive("blockquote")) {
    const { $from } = ed.state.selection;
    if ($from.parent.textContent === "") {
      return ed.commands.lift("blockquote");
    }
    return ed.chain().splitBlock().focus().run();
  }
  return ed.commands.setHardBreak();
}

export const CodeBlockAfterHardBreak = Extension.create({
  name: "codeBlockAfterHardBreak",
  addInputRules() {
    const codeBlockType = this.editor.schema.nodes.codeBlock;
    return [
      new InputRule({
        find: /\n(?:```|~~~)([a-z+]*)[\s]$/,
        handler: ({ state, range, match }) => {
          const $from = state.doc.resolve(range.from);
          if ($from.parent.type.name === "codeBlock") return null;
          const afterParagraph = $from.after();
          const attrs = match[1] ? { language: match[1] } : {};
          state.tr.delete(range.from, range.to);
          const mapped = state.tr.mapping.map(afterParagraph);
          state.tr.insert(mapped, codeBlockType.create(attrs));
          state.tr.setSelection(
            TextSelection.near(state.tr.doc.resolve(mapped + 1)),
          );
        },
      }),
    ];
  },
});
