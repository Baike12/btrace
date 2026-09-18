import { getPgQueue } from "../queueRegistry";

export const getWebhookQueue = () => getPgQueue("webhook-queue");
