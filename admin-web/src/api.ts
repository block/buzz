const PREFIX = "/api/admin/v1";

export class ApiFailure extends Error {
  constructor(
    public status: number,
    message: string,
  ) {
    super(message);
  }
}

export async function request<T>(path: string): Promise<T> {
  const response = await fetch(`${PREFIX}${path}`, {
    credentials: "same-origin",
    headers: { accept: "application/json" },
  });
  if (!response.ok) {
    const envelope = await response.json().catch(() => null);
    throw new ApiFailure(
      response.status,
      envelope?.error?.message ?? `Request failed (${response.status})`,
    );
  }
  return response.json() as Promise<T>;
}
