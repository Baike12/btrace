import { getPgQueue } from "../queueRegistry";

export const getIngestionQueue = () => getPgQueue("ingestion-queue");
