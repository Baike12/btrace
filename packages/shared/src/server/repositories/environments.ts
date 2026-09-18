// PG-only: environments repository
// PG schema doesn't have an "environment" column on traces/observations/scores.
// Return the default environment only.

export type EnvironmentFilterProps = {
  projectId: string;
  fromTimestamp?: Date;
};

export const getEnvironmentsForProject = async (
  _props: EnvironmentFilterProps,
): Promise<{ environment: string }[]> => {
  return [{ environment: "default" }];
};
