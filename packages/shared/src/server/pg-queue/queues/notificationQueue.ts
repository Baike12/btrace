import { getPgQueue } from "../queueRegistry";

export const getNotificationQueue = () => getPgQueue("notification-queue");
