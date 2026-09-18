import { prisma } from "../../db";
import { logger } from "../logger";

/**
 * PG Seen Events — 替代 Redis seen-event cache
 * 用于 ingestion 事件去重（防止重复处理）
 */

/**
 * 检查事件是否已处理过。如果未处理，标记为已见。
 * @returns true 如果已处理（去重命中），false 如果是新事件
 */
export async function checkAndMarkSeenEvent(
  projectId: string,
  eventType: string,
  eventBodyId: string,
  fileKey: string,
): Promise<boolean> {
  const eventKey = `${projectId}:${eventType}:${eventBodyId}:${fileKey}`;

  try {
    await prisma.$executeRawUnsafe(
      `INSERT INTO pg_seen_events (event_key, created_at)
       VALUES ($1, now())
       ON CONFLICT (event_key) DO NOTHING`,
      eventKey,
    );
    // 插入成功 → 新事件
    return false;
  } catch (error) {
    // 插入失败通常是 ON CONFLICT DO NOTHING 导致的，说明已存在
    logger.debug(`Seen event duplicate: ${eventKey}`);
    return true;
  }
}

/**
 * 清理过期的 seen events（10 分钟前）
 */
export async function cleanSeenEvents(): Promise<void> {
  try {
    await prisma.$executeRawUnsafe(
      `DELETE FROM pg_seen_events WHERE created_at < now() - INTERVAL '10 minutes'`,
    );
  } catch (error) {
    logger.error("Failed to clean seen events", error);
  }
}
