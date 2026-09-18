import { getPgQueue } from "../queueRegistry";

export const getEntityChangeQueue = () => getPgQueue("entity-change-queue");
