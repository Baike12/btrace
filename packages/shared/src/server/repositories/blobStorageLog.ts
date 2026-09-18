import {
  queryPg,
  executePg,
  queryPgStream,
} from "./pg";
import { BlobStorageFileRefRecordReadType } from "./definitions";

export const getBlobStorageByProjectAndEntityId = async (
  projectId: string,
  entityType: string,
  entityId: string,
): Promise<BlobStorageFileRefRecordReadType[]> => {
  const query = `
    select *
    from blob_storage_file_log
    where project_id = $1
    and entity_type = $2
    and entity_id = $3
  `;

  return queryPg<BlobStorageFileRefRecordReadType>({
    query,
    params: [projectId, entityType, entityId],
    tags: {
      feature: "eventLog",
      kind: "byID",
      projectId,
    },
  });
};

export const getBlobStorageByProjectId = (
  projectId: string,
): AsyncGenerator<BlobStorageFileRefRecordReadType> => {
  const query = `
    select *
    from blob_storage_file_log
    where project_id = $1
  `;

  return queryPgStream<BlobStorageFileRefRecordReadType>({
    query,
    params: [projectId],
    tags: {
      feature: "eventLog",
      kind: "list",
      projectId,
    },
  });
};

export const getBlobStorageByProjectIdBeforeDate = (
  projectId: string,
  beforeDate: Date,
): AsyncGenerator<BlobStorageFileRefRecordReadType> => {
  const query = `
        select *
        from blob_storage_file_log
        where project_id = $1
        and created_at <= $2::timestamptz
    `;

  return queryPgStream<BlobStorageFileRefRecordReadType>({
    query,
    params: [projectId, beforeDate.toISOString()],
    tags: {
      feature: "eventLog",
      kind: "list",
      projectId,
    },
  });
};

export const getBlobStorageByProjectIdAndEntityIds = (
  projectId: string,
  entityType: "observation" | "trace" | "score",
  entityIds: string[],
): AsyncGenerator<BlobStorageFileRefRecordReadType> => {
  const query = `
    select *
    from blob_storage_file_log
    where project_id = $1
      and entity_type = $2
      and entity_id = ANY($3::text[])
  `;

  return queryPgStream<BlobStorageFileRefRecordReadType>({
    query,
    params: [projectId, entityType, entityIds],
    tags: {
      feature: "eventLog",
      kind: "list",
      projectId,
    },
  });
};

export const getBlobStorageByProjectIdAndTraceIds = (
  projectId: string,
  traceIds: string[],
): AsyncGenerator<BlobStorageFileRefRecordReadType> => {
  const query = `
    with filtered_traces as (
      select distinct
        id as entity_id,
        project_id as project_id,
        'trace' as entity_type
      from traces
      where project_id = $1
        and id = ANY($2::text[])
    ), filtered_observations as (
      select distinct
        id as entity_id,
        project_id as project_id,
        'observation' as entity_type
      from observations
      where project_id = $1
        and trace_id = ANY($2::text[])
    ), filtered_scores as (
      select distinct
        id as entity_id,
        project_id as project_id,
        'score' as entity_type
      from scores
      where project_id = $1
        and trace_id = ANY($2::text[])
    ), filtered_events as (
      select *
      from filtered_traces
      union all
      select *
      from filtered_observations
      union all
      select *
      from filtered_scores
    )

    -- We use exists because we only use the 'filtered_events' as a filter.
    -- There is no need to build the cartesian product (i.e. the combination) between the event log and the events.
    select el.*
    from blob_storage_file_log el
    where el.project_id = $1
    and exists (
      select 1 from filtered_events fe
      where el.project_id = fe.project_id and el.entity_id = fe.entity_id and el.entity_type = fe.entity_type
    )
  `;

  return queryPgStream<BlobStorageFileRefRecordReadType>({
    query,
    params: [projectId, traceIds],
    tags: {
      feature: "eventLog",
      kind: "list",
      projectId,
    },
  });
};

// this function is only used for the background migration from event_log to blob_storage_file_log
export const insertIntoS3RefsTableFromEventLog = async (
  limit: number,
  offset: number,
) => {
  const query = `
    INSERT INTO blob_storage_file_log
    SELECT
      id,
      project_id,
      entity_type,
      entity_id,
      event_id,
      bucket_name,
      bucket_path,
      created_at,
      updated_at,
      created_at AS event_ts,
      0 AS is_deleted
    FROM event_log
    ORDER BY project_id DESC, entity_type DESC, entity_id DESC, bucket_path DESC
    LIMIT $1
    OFFSET $2
  `;

  await executePg({
    query,
    params: [limit, offset],
    tags: {
      feature: "backgroundMigration",
      kind: "list",
    },
  });
};

export const getLastEventLogPrimaryKey = async () => {
  const query = `
    SELECT project_id, entity_type, entity_id, bucket_path
    FROM event_log
    ORDER BY project_id ASC, entity_type ASC, entity_id ASC, bucket_path ASC
    LIMIT 1
  `;
  const result = await queryPg<{
    project_id: string;
    entity_type: string;
    entity_id: string;
    bucket_path: string;
  }>({ query });
  return result.shift();
};

export const findS3RefsByPrimaryKey = async (primaryKey: {
  project_id: string;
  entity_type: string;
  entity_id: string;
  bucket_path: string;
}) => {
  const query = `
    SELECT *
    FROM blob_storage_file_log
    WHERE project_id = $1
      AND entity_type = $2
      AND entity_id = $3
      AND bucket_path = $4
  `;
  return queryPg<BlobStorageFileRefRecordReadType>({
    query,
    params: [primaryKey.project_id, primaryKey.entity_type, primaryKey.entity_id, primaryKey.bucket_path],
  });
};
