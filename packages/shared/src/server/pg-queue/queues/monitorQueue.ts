import { getPgQueue } from "../queueRegistry";

export const getMonitorQueue = () => getPgQueue("monitor-queue");
