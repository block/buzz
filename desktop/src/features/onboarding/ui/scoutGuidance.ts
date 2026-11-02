import type { OnboardingSceneId } from "./onboardingScenes";

export type ScoutPose = "hello" | "listening" | "waiting" | "working" | "done";
export type ScoutTone = "help" | "turn" | "working";
export type ScoutGuidance = {
  title: string;
  copy: string;
  status: string;
  pose: ScoutPose;
  /** Colour of the status dot; the status words decide it when absent. */
  tone?: ScoutTone;
  /** Scout's speech bubble under the copy. */
  bubble?: string;
};

/** Who and what Scout can name in the copy, when the runtime knows. */
export type ScoutContext = {
  /** First name of the person, for a personalized greeting. */
  firstName?: string;
  /** The AI app in use, for "I'm starting up on Claude Code". */
  appLabel?: string;
  /** Business named by an invite. */
  businessName?: string;
  /** Person who sent the invite, when the server says. */
  inviter?: string;
};

export function scoutTone(status: string): ScoutTone {
  if (status === "Working") return "working";
  if (status === "Here to help") return "help";
  return "turn";
}

const HELP = "Here to help";
const TURN = "Your turn";
const WORKING = "Working";
const ATTENTION = "Needs your attention";

/** Presentation state only. Runtime owners supply working, feedback and reply evidence. */
export function scoutGuidance(
  scene: OnboardingSceneId,
  error?: string | null,
  override?: ScoutGuidance,
  _focusTopic?: string,
  connectionChecking = false,
  context: ScoutContext = {},
): ScoutGuidance {
  if (override) return override;
  let guidance: ScoutGuidance = {
    title: "Welcome. Let’s create a colony.",
    copy: "I’m Scout, your Chief of Staff. I’ll help you get set up.",
    status: HELP,
    pose: "hello",
    bubble: scene === "account" ? "What should I call you?" : undefined,
  };
  if (scene.includes("business") || scene === "additional")
    guidance = {
      title: "Now, tell me about the business.",
      copy: "I’ll shape your workspace around it. You can change any of this later.",
      status: HELP,
      pose: "hello",
    };
  else if (/subscription|credits|openrouter|connect|funding/.test(scene))
    guidance = {
      title: "Choose how I think.",
      copy: "I run on the AI you connect. Use an app you already have, or start with Colony Agent. You can switch at any time.",
      status: TURN,
      pose: "waiting",
    };
  if (/signin|forgot|reset|new-password|email-sent/.test(scene))
    guidance = {
      title: "Let’s get you back in.",
      copy: "Your business and your team will be right where you left them.",
      status: HELP,
      pose: "listening",
    };
  if (/verify|code/.test(scene))
    guidance = {
      title: context.firstName
        ? `Nice to meet you, ${context.firstName}.`
        : "Check your email.",
      copy: "I’ve sent a six-digit code to your inbox. It keeps your account yours, on any device.",
      status: TURN,
      pose: "waiting",
    };
  if (scene === "invite")
    guidance = {
      title: "Come on in.",
      copy: context.businessName
        ? `${context.inviter ? `${context.inviter} invited` : "You’ve been invited"}${context.inviter ? " you to" : " to"} ${context.businessName}. Once you’ve joined, I’ll help you find your feet.`
        : "You’ve been invited. Once you’ve joined, I’ll help you find your feet.",
      status: HELP,
      pose: "hello",
    };
  if (scene === "businesses")
    guidance = {
      title: context.firstName
        ? `Welcome back, ${context.firstName}.`
        : "Welcome back.",
      copy: "Pick a business and I’ll catch you up on what moved while you were away.",
      status: HELP,
      pose: "hello",
    };
  const working =
    scene === "testing" ||
    scene === "subscription-scan" ||
    scene === "credits-pending" ||
    scene === "community-entry" ||
    connectionChecking;
  if (working)
    guidance =
      scene === "testing"
        ? {
            title: "Saying my first hello.",
            copy: `I’m starting up on ${context.appLabel || "your AI"} and sending you a message. This usually takes a few seconds.`,
            status: WORKING,
            pose: "working",
          }
        : scene === "community-entry"
          ? {
              title: "Setting up your home.",
              copy: "I’m putting up the Welcome channel and getting ready to say hello. Almost there.",
              status: WORKING,
              pose: "working",
            }
          : {
              title:
                scene === "subscription-scan" || connectionChecking
                  ? "Checking your installed apps."
                  : "Getting things ready.",
              copy: "You can see the progress alongside me.",
              status: WORKING,
              pose: "working",
            };
  const inlineErrorScenes: OnboardingSceneId[] = [
    "account-error",
    "business-error",
    "connection-error",
    "community-entry-error",
    "credits-failed",
    "credits-price-error",
    "openrouter-error",
    "openrouter-limit",
    "subscription-models-error",
    "subscription-exhausted",
  ];
  const codeFeedback = scene.startsWith("verify") || scene.startsWith("reset");
  if (
    ((error && !codeFeedback) || inlineErrorScenes.includes(scene)) &&
    !working
  )
    guidance = {
      title: "We can sort this out.",
      copy: "Your next step is shown alongside me. You can retry or use another option.",
      status: ATTENTION,
      pose: "waiting",
    };
  if (scene === "connected")
    guidance = {
      title: "We’re connected.",
      copy: "I’ll be waiting in your Welcome channel. Bring me something you’re working on, or give me a quick challenge.",
      status: HELP,
      pose: "done",
    };
  return { ...guidance, tone: guidance.tone ?? scoutTone(guidance.status) };
}
