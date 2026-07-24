/**
 * Frontend-only stubs for the onboarding v2 "standard flow" backend seams.
 *
 * The signup-first onboarding (see PLANS/ONBOARDING_V2_STANDARD_FLOW.md in the
 * workspace) needs endpoints that do not exist on the relay yet: email/password
 * accounts, invitee-bound community invites, in-app community creation, and
 * email invites. This module is the typed contract for those endpoints; every
 * function currently resolves fixture data after a short delay so the whole
 * flow is walkable in the UI with zero backend changes.
 *
 * Engineers: replace the bodies (not the signatures) when the real endpoints
 * land. Error cases are expressed as thrown `OnboardingAccountError`s with
 * stable `code` values the UI already branches on.
 */

export type OnboardingAccountErrorCode =
  | "email_invalid"
  | "email_taken"
  | "password_weak"
  | "credentials_invalid"
  | "community_name_taken"
  | "network";

export class OnboardingAccountError extends Error {
  readonly code: OnboardingAccountErrorCode;

  constructor(code: OnboardingAccountErrorCode, message: string) {
    super(message);
    this.name = "OnboardingAccountError";
    this.code = code;
  }
}

export type InvitedCommunity = {
  /** Opaque invite identifier used to claim the invite. */
  inviteId: string;
  /** Human-readable community name, e.g. "The Land of Ooo". */
  name: string;
  /** Community hostname shown under the name in the hub list. */
  host: string;
  /** Relay websocket URL to connect to on accept. */
  relayWsUrl: string;
};

export type CreatedCommunity = {
  name: string;
  relayWsUrl: string;
};

const EMAIL_PATTERN = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;
/** Minimum password length enforced client-side until the backend owns policy. */
export const MIN_PASSWORD_LENGTH = 8;

function delay(ms: number) {
  return new Promise<void>((resolve) => setTimeout(resolve, ms));
}

/** Client-side email shape check shared by the signup and login forms. */
export function isValidEmail(email: string): boolean {
  return EMAIL_PATTERN.test(email.trim());
}

/**
 * Create an email/password account. In the real flow this pairs the account
 * with the freshly generated identity key; the stub only validates shape.
 */
export async function signUp({
  email,
  password,
}: {
  email: string;
  password: string;
}): Promise<{ ok: true }> {
  await delay(400);
  if (!isValidEmail(email)) {
    throw new OnboardingAccountError(
      "email_invalid",
      "Enter a valid email address.",
    );
  }
  if (password.length < MIN_PASSWORD_LENGTH) {
    throw new OnboardingAccountError(
      "password_weak",
      `Password must be at least ${MIN_PASSWORD_LENGTH} characters.`,
    );
  }
  return { ok: true };
}

/** Log in a returning email/password user. Stub accepts any valid shape. */
export async function logIn({
  email,
  password,
}: {
  email: string;
  password: string;
}): Promise<{ ok: true }> {
  await delay(400);
  if (!isValidEmail(email) || password.length === 0) {
    throw new OnboardingAccountError(
      "credentials_invalid",
      "Email or password is incorrect.",
    );
  }
  return { ok: true };
}

/**
 * Communities the current account has been invited to. Invites are bound to
 * the invitee server-side, so onboarding can list them without a pasted link.
 */
export async function listInvitedCommunities(): Promise<InvitedCommunity[]> {
  await delay(300);
  return [
    {
      inviteId: "stub-invite-1",
      name: "The Land of Ooo",
      host: "land-of-ooo.communities.buzz.xyz",
      relayWsUrl: "wss://land-of-ooo.communities.buzz.xyz",
    },
  ];
}

/** Accept an invitee-bound invite and return the relay to connect to. */
export async function connectInvitedCommunity(
  inviteId: string,
): Promise<{ relayWsUrl: string }> {
  await delay(300);
  const invites = await listInvitedCommunities();
  const invite = invites.find((candidate) => candidate.inviteId === inviteId);
  if (!invite) {
    throw new OnboardingAccountError("network", "Invite is no longer valid.");
  }
  return { relayWsUrl: invite.relayWsUrl };
}

/** Create a new hosted community from inside onboarding. */
export async function createCommunity({
  name,
  description,
  avatarUrl,
}: {
  name: string;
  description: string;
  avatarUrl?: string;
}): Promise<CreatedCommunity> {
  await delay(500);
  void description;
  void avatarUrl;
  const trimmed = name.trim();
  if (!trimmed) {
    throw new OnboardingAccountError(
      "community_name_taken",
      "Enter a community name.",
    );
  }
  const slug = trimmed
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "");
  return {
    name: trimmed,
    relayWsUrl: `wss://${slug || "community"}.communities.buzz.xyz`,
  };
}

/** Send email invites to join the newly created community. */
export async function sendCommunityInvites({
  emails,
}: {
  emails: string[];
}): Promise<{ sent: number }> {
  await delay(300);
  const valid = emails.filter((email) => isValidEmail(email));
  return { sent: valid.length };
}
