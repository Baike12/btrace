import { type FilterState } from "../../../types";
import {
  type ColumnDefinition,
  type UiColumnMappings,
} from "../../../tableDefinitions";
import {
  StringFilter,
  DateTimeFilter,
  NumberFilter,
  ArrayOptionsFilter,
  BooleanFilter,
  NullFilter,
  StringOptionsFilter,
  FilterList,
} from "./pg-filter";

/**
 * createFilterFromFilterState — PG 版本
 * 将前端 FilterState 转换为 PG SQL WHERE 条件。
 * 保持与 ClickHouse 版本相同的函数签名和返回类型。
 */
export function createFilterFromFilterState(
  filter: FilterState,
  columnMapping?: UiColumnMappings,
  columnDefinitions?: ColumnDefinition[],
): (StringFilter | DateTimeFilter | NumberFilter | ArrayOptionsFilter | BooleanFilter | NullFilter | StringOptionsFilter)[] {
  return filter.map((f) => {
    const field = f.column;
    const tableName = "traces";

    switch (f.type) {
      case "string":
        return new StringFilter({
          clickhouseTable: tableName,
          field,
          operator: f.operator,
          value: f.value,
        });
      case "datetime":
        return new DateTimeFilter({
          clickhouseTable: tableName,
          field,
          operator: f.operator,
          value: new Date(f.value),
        });
      case "number":
        return new NumberFilter({
          clickhouseTable: tableName,
          field,
          operator: f.operator,
          value: f.value,
        });
      case "arrayOptions":
        return new ArrayOptionsFilter({
          clickhouseTable: tableName,
          field,
          operator: f.operator,
          values: f.value,
        });
      case "boolean":
        return new BooleanFilter({
          clickhouseTable: tableName,
          field,
          operator: f.operator,
          value: f.value,
        });
      case "null":
        return new NullFilter({
          clickhouseTable: tableName,
          field,
          operator: f.operator,
        });
      case "stringOptions":
        return new StringOptionsFilter({
          clickhouseTable: tableName,
          field,
          operator: f.operator,
          values: f.value,
        });
      default:
        throw new Error(`Unsupported filter type: ${(f as any).type}`);
    }
  });
}

/**
 * getProjectIdDefaultFilter — 创建默认 project_id 过滤器
 * 返回包含 project_id 基础过滤的 FilterList
 */
export function getProjectIdDefaultFilter(
  projectId: string,
  opts: { tracesPrefix?: string; scoresPrefix?: string } = {},
): { tracesFilter: FilterList; scoresFilter?: FilterList } {
  const tracesFilter = new FilterList([
    new StringFilter({
      clickhouseTable: "traces",
      field: "project_id",
      operator: "=",
      value: projectId,
      tablePrefix: opts.tracesPrefix,
    }),
  ]);

  let scoresFilter: FilterList | undefined;
  if (opts.scoresPrefix) {
    scoresFilter = new FilterList([
      new StringFilter({
        clickhouseTable: "scores",
        field: "project_id",
        operator: "=",
        value: projectId,
        tablePrefix: opts.scoresPrefix,
      }),
    ]);
  }

  return { tracesFilter, scoresFilter };
}
