import { prisma } from "../../db";

/**
 * PG Cache — 替代 Redis 缓存的通用实现
 */

export async function cacheGet<T>(key: string): Promise<T | null> {
  try {
    const rows = await prisma.$queryRawUnsafe<{ value: T }[]>(
      `SELECT value FROM pg_cache
       WHERE cache_key = $1 AND expires_at > now()`,
      key,
    );
    return rows.length > 0 ? (rows[0].value as T) : null;
  } catch {
    return null;
  }
}

export async function cacheSet<T>(
  key: string,
  value: T,
  ttlSeconds: number,
): Promise<void> {
  try {
    await prisma.$executeRawUnsafe(
      `INSERT INTO pg_cache (cache_key, value, expires_at)
       VALUES ($1, $2::jsonb, now() + INTERVAL '1 second' * $3)
       ON CONFLICT (cache_key) DO UPDATE SET
         value = EXCLUDED.value,
         expires_at = EXCLUDED.expires_at`,
      key,
      JSON.stringify(value),
      ttlSeconds,
    );
  } catch {
    // 缓存不阻塞业务
  }
}

export async function cacheDelete(key: string): Promise<void> {
  try {
    await prisma.$executeRawUnsafe(
      `DELETE FROM pg_cache WHERE cache_key = $1`,
      key,
    );
  } catch {
    // ignore
  }
}

export async function cleanExpiredCache(): Promise<void> {
  try {
    await prisma.$executeRawUnsafe(
      `DELETE FROM pg_cache WHERE expires_at < now()`,
    );
  } catch {
    // ignore
  }
}
