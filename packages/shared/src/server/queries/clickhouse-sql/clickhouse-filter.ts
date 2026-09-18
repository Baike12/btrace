// Stub: re-export from pg-sql equivalents for PG-only fork
export {
  type Filter,
  FilterList,
  StringFilter,
  DateTimeFilter,
  StringOptionsFilter,
  NumberFilter,
  ArrayOptionsFilter,
  BooleanFilter,
  NullFilter,
} from "../pg-sql/pg-filter";

// ClickhouseOperator alias for backward compat
export type ClickhouseOperator = "any of" | "none of" | string;

// Stub classes not in pg-filter
export class CategoryOptionsFilter {
  constructor(public column: string, public operator: string, public value: string[]) {}
}
export class NumberObjectFilter {
  constructor(public column: string, public operator: string, public value: Record<string, number>) {}
}
export class StringObjectFilter {
  constructor(public column: string, public operator: string, public value: Record<string, string>) {}
}
