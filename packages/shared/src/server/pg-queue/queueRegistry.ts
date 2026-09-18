import { PgQueue } from "./PgQueue";

/**
 * 队列注册表 — 管理所有 PgQueue 单例
 */
const queueInstances = new Map<string, PgQueue>();

export function getPgQueue(queueName: string): PgQueue {
  if (!queueInstances.has(queueName)) {
    queueInstances.set(queueName, new PgQueue(queueName));
  }
  return queueInstances.get(queueName)!;
}

/**
 * 关闭所有队列（清理）
 */
export async function closeAllQueues(): Promise<void> {
  queueInstances.clear();
}
