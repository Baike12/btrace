import { randomUUID } from "crypto";
import { prisma } from "../../db";
import { logger } from "../logger";

export interface PgJobOptions {
  delay?: number;
  attempts?: number;
  backoff?: {
    type: "fixed" | "exponential";
    delay: number;
  };
}

/**
 * PgQueue — 替代 BullMQ Queue 的 PG 队列生产者
 */
export class PgQueue<TData = any> {
  public readonly queueName: string;

  constructor(queueName: string) {
    this.queueName = queueName;
  }

  /**
   * 添加作业到队列
   */
  async add(
    name: string,
    data: TData,
    opts?: PgJobOptions,
  ): Promise<string> {
    const jobId = randomUUID();
    const delay = opts?.delay ?? 0;
    const runAt = delay > 0
      ? new Date(Date.now() + delay)
      : new Date();

    const payload = typeof data === "string"
      ? data
      : JSON.parse(JSON.stringify(data)); // deep clone + serialize

    try {
      await prisma.$executeRawUnsafe(
        `INSERT INTO pg_jobs (job_id, queue_name, payload, state, run_at,
                              max_attempts, backoff_type, backoff_delay)
         VALUES ($1, $2, $3::jsonb, 'waiting', $4, $5, $6, $7)`,
        jobId,
        this.queueName,
        JSON.stringify(payload),
        runAt,
        opts?.attempts ?? 5,
        opts?.backoff?.type ?? "exponential",
        opts?.backoff?.delay ?? 5000,
      );
      return jobId;
    } catch (error) {
      logger.error(`PgQueue.add failed for ${this.queueName}`, error);
      throw error;
    }
  }

  /**
   * 获取队列中的作业数
   */
  async count(): Promise<number> {
    const result = await prisma.$queryRawUnsafe<{ count: bigint }[]>(
      `SELECT count(*) as count FROM pg_jobs WHERE queue_name = $1 AND state = 'waiting'`,
      this.queueName,
    );
    return Number(result[0]?.count ?? 0);
  }

  /**
   * 暂停队列（将 waiting 作业都标记为 delayed）
   */
  async pause(): Promise<void> {
    await prisma.$executeRawUnsafe(
      `UPDATE pg_jobs SET state = 'delayed' WHERE queue_name = $1 AND state = 'waiting'`,
      this.queueName,
    );
  }

  /**
   * 恢复队列（将 delayed 作业标记回 waiting）
   */
  async resume(): Promise<void> {
    await prisma.$executeRawUnsafe(
      `UPDATE pg_jobs SET state = 'waiting' WHERE queue_name = $1 AND state = 'delayed'`,
      this.queueName,
    );
  }

  /**
   * 清理已完成/失败的作业
   */
  async clean(graceMs: number): Promise<void> {
    const cutoff = new Date(Date.now() - graceMs);
    await prisma.$executeRawUnsafe(
      `DELETE FROM pg_jobs
       WHERE queue_name = $1
         AND state IN ('completed', 'failed')
         AND finished_at < $2`,
      this.queueName,
      cutoff,
    );
  }
}
