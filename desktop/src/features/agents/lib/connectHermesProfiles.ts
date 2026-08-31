export type HermesConnectSuccess<T> = {
  profile: string;
  value: T;
};

export type HermesConnectFailure = {
  profile: string;
  error: string;
};

/** Connect independent local profiles with a bounded startup fan-out. */
export async function connectHermesProfiles<T>({
  profiles,
  connect,
  concurrency = 2,
}: {
  profiles: readonly string[];
  connect: (profile: string) => Promise<T>;
  concurrency?: number;
}): Promise<{
  successes: HermesConnectSuccess<T>[];
  failures: HermesConnectFailure[];
}> {
  if (profiles.length === 0) return { successes: [], failures: [] };
  const width = Math.max(1, Math.min(Math.floor(concurrency), profiles.length));
  const successes: HermesConnectSuccess<T>[] = [];
  const failures: HermesConnectFailure[] = [];
  let nextIndex = 0;

  async function worker() {
    while (nextIndex < profiles.length) {
      const profile = profiles[nextIndex];
      nextIndex += 1;
      try {
        successes.push({ profile, value: await connect(profile) });
      } catch (cause) {
        failures.push({
          profile,
          error:
            cause instanceof Error
              ? cause.message
              : "Could not connect profile.",
        });
      }
    }
  }

  await Promise.all(Array.from({ length: width }, () => worker()));
  return { successes, failures };
}
