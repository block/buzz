/** A compact, cited summary of a BlockUI decision and its limits. */
export interface GrammarSection {
  id: string;
  title: string;
  sources: string[];
  rows: Array<[choice: string, use: string, boundary: string]>;
}

/** Verified upstream snapshot; reference links never load assets into the catalog. */
export const BLOCKUI_REFERENCE =
  "https://github.com/squareup/design-blockinterface/blob/c89319ea3e11a35d58f19ba83f4d976d143e7485/blockUI/";
