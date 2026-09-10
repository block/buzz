// Fixed, small BW issue templates (P4D). This is deliberately not a template
// engine: three hardcoded shapes, each a description skeleton plus default
// acceptance criteria and optional non-goals the reporter edits before
// submit. Adding a fourth kind means adding a fourth literal entry here, not
// a new abstraction.
export type BwIssueTemplateId = "bug" | "feature" | "operations";

export type BwIssueTemplate = {
  id: BwIssueTemplateId;
  label: string;
  descriptionSkeleton: string;
  defaultAcceptanceCriteria: string[];
  defaultNonGoals: string[];
};

export const BW_ISSUE_TEMPLATES: BwIssueTemplate[] = [
  {
    id: "bug",
    label: "Bug",
    descriptionSkeleton:
      "Steps to reproduce:\n1. \n2. \n\nExpected:\n\nActual:\n",
    defaultAcceptanceCriteria: [
      "The reported defect no longer reproduces with the steps above",
    ],
    defaultNonGoals: [],
  },
  {
    id: "feature",
    label: "Feature",
    descriptionSkeleton: "Problem:\n\nProposed change:\n",
    defaultAcceptanceCriteria: [
      "The described behavior is implemented and covered by a test",
    ],
    defaultNonGoals: [],
  },
  {
    id: "operations",
    label: "Operations",
    descriptionSkeleton: "Operational task:\n\nWhy now:\n",
    defaultAcceptanceCriteria: [
      "The operational task is completed and verified",
    ],
    defaultNonGoals: [],
  },
];

export function bwIssueTemplateById(
  id: string | null | undefined,
): BwIssueTemplate | null {
  return BW_ISSUE_TEMPLATES.find((template) => template.id === id) ?? null;
}
