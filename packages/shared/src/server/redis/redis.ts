// Stub for PG-only fork: no Redis client needed
export const redis = null as unknown as {
  get: (key: string) => Promise<string | null>;
  set: (key: string, value: string, mode?: string, duration?: number) => Promise<"OK" | null>;
  del: (key: string) => Promise<number>;
  expire: (key: string, seconds: number) => Promise<number>;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  sadd: (key: string, ...members: string[]) => Promise<number>;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  srem: (key: string, ...members: string[]) => Promise<number>;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  smembers: (key: string) => Promise<string[]>;
};
