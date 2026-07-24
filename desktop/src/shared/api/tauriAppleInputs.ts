import { invokeTauri } from "@/shared/api/tauri";

export type AppleInputSource = "calendar" | "reminders" | "notes" | "files";
export type AppleInputPermission =
  | "not_determined"
  | "denied"
  | "authorized"
  | "restricted"
  | "unavailable";

export type AppleInputRequest =
  | {
      operation: "permission_status";
      arguments: { source: AppleInputSource };
    }
  | {
      operation: "request_permission";
      arguments: { source: "calendar" | "reminders" };
    }
  | {
      operation: "read_calendar";
      arguments: {
        calendar_ids: string[];
        start: string;
        end: string;
        maximum: number;
      };
    }
  | {
      operation: "read_reminders";
      arguments: {
        list_ids: string[];
        start: string;
        end: string;
        maximum: number;
      };
    }
  | {
      operation: "read_notes";
      arguments: { folder_ids: string[]; maximum: number };
    }
  | {
      operation: "read_files";
      arguments: { paths: string[] };
    };

export type AppleInputResponse = {
  source: AppleInputSource;
  permission: AppleInputPermission;
  observedAt: string;
  records: Array<{ fields: Record<string, string> }>;
  truncated: boolean;
  error: string | null;
};

export function readAppleInputs(
  request: AppleInputRequest,
): Promise<AppleInputResponse> {
  return invokeTauri<AppleInputResponse>("read_apple_inputs", {
    request,
  });
}
