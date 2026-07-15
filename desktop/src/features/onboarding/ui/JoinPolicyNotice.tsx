import * as React from "react";

import type { JoinPolicy } from "@/shared/api/invites";
import { Button } from "@/shared/ui/button";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import { Markdown } from "@/shared/ui/markdown";

type JoinPolicyNoticeProps = {
  ageConfirmed: boolean;
  onAgeConfirmedChange: (confirmed: boolean) => void;
  policy: JoinPolicy;
};

export function JoinPolicyNotice({
  ageConfirmed,
  onAgeConfirmedChange,
  policy,
}: JoinPolicyNoticeProps) {
  const [openDocument, setOpenDocument] = React.useState<
    "terms" | "privacy" | null
  >(null);
  const markdown =
    openDocument === "terms" ? policy.termsMarkdown : policy.privacyMarkdown;

  return (
    <div className="space-y-3 rounded-md border p-3 text-left text-sm">
      {policy.ageAttestationRequired ? (
        <label className="flex items-start gap-2">
          <input
            checked={ageConfirmed}
            className="mt-1"
            onChange={(event) => onAgeConfirmedChange(event.target.checked)}
            type="checkbox"
          />
          <span>I am 18 years of age or older.</span>
        </label>
      ) : null}

      {policy.termsMarkdown || policy.privacyMarkdown ? (
        <p className="text-muted-foreground">
          By proceeding you agree to the Buzz{" "}
          {policy.termsMarkdown ? (
            <Button
              className="h-auto p-0 align-baseline"
              onClick={() => setOpenDocument("terms")}
              type="button"
              variant="link"
            >
              Terms of Service
            </Button>
          ) : null}
          {policy.termsMarkdown && policy.privacyMarkdown ? " and " : null}
          {policy.privacyMarkdown ? (
            <Button
              className="h-auto p-0 align-baseline"
              onClick={() => setOpenDocument("privacy")}
              type="button"
              variant="link"
            >
              Privacy Policy
            </Button>
          ) : null}
          .
        </p>
      ) : null}

      <Dialog
        onOpenChange={(open) => {
          if (!open) setOpenDocument(null);
        }}
        open={openDocument !== null}
      >
        <DialogContent className="max-h-[80vh] max-w-2xl overflow-y-auto">
          <DialogHeader>
            <DialogTitle>
              {openDocument === "terms" ? "Terms of Service" : "Privacy Policy"}
            </DialogTitle>
          </DialogHeader>
          {markdown ? <Markdown content={markdown} /> : null}
        </DialogContent>
      </Dialog>
    </div>
  );
}
