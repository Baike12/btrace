import {
  filterOperators,
  type FtsMatchOperator,
} from "../../../interfaces/filters";
import { escapeSqlLikePattern } from "../../utils/sqlLike";
import { pgCompliantRandomCharacters } from "../../repositories/pg";

export type PgOperator =
  | (typeof filterOperators)[keyof typeof filterOperators][number]
  | "!="
  | FtsMatchOperator;

export interface PgFilterResult {
  query: string;
  params: unknown[];
}

export interface Filter {
  apply(paramIndex: number): PgFilterResult & { nextParamIndex: number };
  clickhouseTable: string;
  tablePrefix?: string;
  operator: PgOperator;
  field: string;
}

// ============================================================
// StringFilter — PG 版本
// ============================================================
export class StringFilter implements Filter {
  public clickhouseTable: string;
  public field: string;
  public value: string;
  public operator:
    | (typeof filterOperators)["string"][number]
    | FtsMatchOperator
    | "!=";
  public tablePrefix?: string;
  public emptyEqualsNull?: boolean;

  constructor(opts: {
    clickhouseTable: string;
    field: string;
    operator: (typeof filterOperators)["string"][number] | FtsMatchOperator | "!=";
    value: string;
    tablePrefix?: string;
    emptyEqualsNull?: boolean;
  }) {
    this.clickhouseTable = opts.clickhouseTable;
    this.field = opts.field;
    this.value = opts.value;
    this.operator = opts.operator;
    this.tablePrefix = opts.tablePrefix;
    this.emptyEqualsNull = opts.emptyEqualsNull;
  }

  apply(paramIndex: number): PgFilterResult & { nextParamIndex: number } {
    const fieldWithPrefix = this.tablePrefix
      ? `"${this.tablePrefix}"."${this.field}"`
      : `"${this.field}"`;

    // '' ≡ NULL: match both '' and NULL
    if (this.emptyEqualsNull && this.value === "") {
      if (
        this.operator === "=" ||
        this.operator === "contains" ||
        this.operator === "starts with" ||
        this.operator === "ends with"
      ) {
        return {
          query: `(${fieldWithPrefix} = '' OR ${fieldWithPrefix} IS NULL)`,
          params: [],
          nextParamIndex: paramIndex,
        };
      }
    }

    const $p = paramIndex;

    switch (this.operator) {
      case "=":
        return {
          query: `${fieldWithPrefix} = $${$p}::text`,
          params: [this.value],
          nextParamIndex: $p + 1,
        };
      case "!=":
        return {
          query: `${fieldWithPrefix} != $${$p}::text`,
          params: [this.value],
          nextParamIndex: $p + 1,
        };
      case "contains":
        return {
          query: `${fieldWithPrefix} LIKE $${$p}::text`,
          params: [`%${escapeSqlLikePattern(this.value)}%`],
          nextParamIndex: $p + 1,
        };
      case "does not contain":
        return {
          query: `${fieldWithPrefix} NOT LIKE $${$p}::text`,
          params: [`%${escapeSqlLikePattern(this.value)}%`],
          nextParamIndex: $p + 1,
        };
      case "starts with":
        return {
          query: `${fieldWithPrefix} LIKE $${$p}::text`,
          params: [`${escapeSqlLikePattern(this.value)}%`],
          nextParamIndex: $p + 1,
        };
      case "ends with":
        return {
          query: `${fieldWithPrefix} LIKE $${$p}::text`,
          params: [`%${escapeSqlLikePattern(this.value)}`],
          nextParamIndex: $p + 1,
        };
      default:
        throw new Error(`Unsupported string operator: ${this.operator}`);
    }
  }
}

// ============================================================
// DateTimeFilter — PG 版本
// ============================================================
export class DateTimeFilter implements Filter {
  public clickhouseTable: string;
  public field: string;
  public value: Date;
  public operator: (typeof filterOperators)["datetime"][number];
  public tablePrefix?: string;

  constructor(opts: {
    clickhouseTable: string;
    field: string;
    operator: (typeof filterOperators)["datetime"][number];
    value: Date;
    tablePrefix?: string;
  }) {
    this.clickhouseTable = opts.clickhouseTable;
    this.field = opts.field;
    this.value = opts.value;
    this.operator = opts.operator;
    this.tablePrefix = opts.tablePrefix;
  }

  apply(paramIndex: number): PgFilterResult & { nextParamIndex: number } {
    const fieldWithPrefix = this.tablePrefix
      ? `"${this.tablePrefix}"."${this.field}"`
      : `"${this.field}"`;
    const $p = paramIndex;
    const val = this.value.toISOString();

    switch (this.operator) {
      case ">":
        return { query: `${fieldWithPrefix} > $${$p}::timestamptz`, params: [val], nextParamIndex: $p + 1 };
      case ">=":
        return { query: `${fieldWithPrefix} >= $${$p}::timestamptz`, params: [val], nextParamIndex: $p + 1 };
      case "<":
        return { query: `${fieldWithPrefix} < $${$p}::timestamptz`, params: [val], nextParamIndex: $p + 1 };
      case "<=":
        return { query: `${fieldWithPrefix} <= $${$p}::timestamptz`, params: [val], nextParamIndex: $p + 1 };
      default:
        throw new Error(`Unsupported datetime operator: ${this.operator}`);
    }
  }
}

// ============================================================
// NumberFilter — PG 版本
// ============================================================
export class NumberFilter implements Filter {
  public clickhouseTable: string;
  public field: string;
  public value: number;
  public operator: (typeof filterOperators)["number"][number];
  public tablePrefix?: string;

  constructor(opts: {
    clickhouseTable: string;
    field: string;
    operator: (typeof filterOperators)["number"][number];
    value: number;
    tablePrefix?: string;
  }) {
    this.clickhouseTable = opts.clickhouseTable;
    this.field = opts.field;
    this.value = opts.value;
    this.operator = opts.operator;
    this.tablePrefix = opts.tablePrefix;
  }

  apply(paramIndex: number): PgFilterResult & { nextParamIndex: number } {
    const fieldWithPrefix = this.tablePrefix
      ? `"${this.tablePrefix}"."${this.field}"`
      : `"${this.field}"`;
    const $p = paramIndex;

    switch (this.operator) {
      case "=":
        return { query: `${fieldWithPrefix} = $${$p}::float8`, params: [this.value], nextParamIndex: $p + 1 };
      case ">":
        return { query: `${fieldWithPrefix} > $${$p}::float8`, params: [this.value], nextParamIndex: $p + 1 };
      case ">=":
        return { query: `${fieldWithPrefix} >= $${$p}::float8`, params: [this.value], nextParamIndex: $p + 1 };
      case "<":
        return { query: `${fieldWithPrefix} < $${$p}::float8`, params: [this.value], nextParamIndex: $p + 1 };
      case "<=":
        return { query: `${fieldWithPrefix} <= $${$p}::float8`, params: [this.value], nextParamIndex: $p + 1 };
      default:
        throw new Error(`Unsupported number operator: ${this.operator}`);
    }
  }
}

// ============================================================
// ArrayOptionsFilter — PG 版本
// ============================================================
export class ArrayOptionsFilter implements Filter {
  public clickhouseTable: string;
  public field: string;
  public values: string[];
  public operator: (typeof filterOperators)["arrayOptions"][number];
  public tablePrefix?: string;

  constructor(opts: {
    clickhouseTable: string;
    field: string;
    operator: (typeof filterOperators)["arrayOptions"][number];
    values: string[];
    tablePrefix?: string;
  }) {
    this.clickhouseTable = opts.clickhouseTable;
    this.field = opts.field;
    this.values = opts.values;
    this.operator = opts.operator;
    this.tablePrefix = opts.tablePrefix;
  }

  apply(paramIndex: number): PgFilterResult & { nextParamIndex: number } {
    const fieldWithPrefix = this.tablePrefix
      ? `"${this.tablePrefix}"."${this.field}"`
      : `"${this.field}"`;

    switch (this.operator) {
      case "any of":
        // PG: val = ANY(tags)
        return {
          query: `${fieldWithPrefix} && $${paramIndex}::text[]`,
          params: [this.values],
          nextParamIndex: paramIndex + 1,
        };
      case "none of":
        return {
          query: `NOT (${fieldWithPrefix} && $${paramIndex}::text[])`,
          params: [this.values],
          nextParamIndex: paramIndex + 1,
        };
      case "all of":
        // PG: tags @> ARRAY['a','b']
        return {
          query: `${fieldWithPrefix} @> $${paramIndex}::text[]`,
          params: [this.values],
          nextParamIndex: paramIndex + 1,
        };
      default:
        throw new Error(`Unsupported arrayOptions operator: ${this.operator}`);
    }
  }
}

// ============================================================
// BooleanFilter — PG 版本
// ============================================================
export class BooleanFilter implements Filter {
  public clickhouseTable: string;
  public field: string;
  public value: boolean;
  public operator: (typeof filterOperators)["boolean"][number];
  public tablePrefix?: string;

  constructor(opts: {
    clickhouseTable: string;
    field: string;
    operator: (typeof filterOperators)["boolean"][number];
    value: boolean;
    tablePrefix?: string;
  }) {
    this.clickhouseTable = opts.clickhouseTable;
    this.field = opts.field;
    this.value = opts.value;
    this.operator = opts.operator;
    this.tablePrefix = opts.tablePrefix;
  }

  apply(paramIndex: number): PgFilterResult & { nextParamIndex: number } {
    const fieldWithPrefix = this.tablePrefix
      ? `"${this.tablePrefix}"."${this.field}"`
      : `"${this.field}"`;
    const $p = paramIndex;

    return {
      query: `${fieldWithPrefix} = $${$p}::boolean`,
      params: [this.value],
      nextParamIndex: $p + 1,
    };
  }
}

// ============================================================
// NullFilter — PG 版本
// ============================================================
export class NullFilter implements Filter {
  public clickhouseTable: string;
  public field: string;
  public operator: (typeof filterOperators)["null"][number];
  public tablePrefix?: string;

  constructor(opts: {
    clickhouseTable: string;
    field: string;
    operator: (typeof filterOperators)["null"][number];
    tablePrefix?: string;
  }) {
    this.clickhouseTable = opts.clickhouseTable;
    this.field = opts.field;
    this.operator = opts.operator;
    this.tablePrefix = opts.tablePrefix;
  }

  apply(paramIndex: number): PgFilterResult & { nextParamIndex: number } {
    const fieldWithPrefix = this.tablePrefix
      ? `"${this.tablePrefix}"."${this.field}"`
      : `"${this.field}"`;

    switch (this.operator) {
      case "is null":
        return { query: `${fieldWithPrefix} IS NULL`, params: [], nextParamIndex: paramIndex };
      case "is not null":
        return { query: `${fieldWithPrefix} IS NOT NULL`, params: [], nextParamIndex: paramIndex };
      default:
        throw new Error(`Unsupported null operator: ${this.operator}`);
    }
  }
}

// ============================================================
// StringOptionsFilter — PG 版本
// ============================================================
export class StringOptionsFilter implements Filter {
  public clickhouseTable: string;
  public field: string;
  public values: string[];
  public operator: (typeof filterOperators)["stringOptions"][number];
  public tablePrefix?: string;

  constructor(opts: {
    clickhouseTable: string;
    field: string;
    operator: (typeof filterOperators)["stringOptions"][number];
    values: string[];
    tablePrefix?: string;
  }) {
    this.clickhouseTable = opts.clickhouseTable;
    this.field = opts.field;
    this.values = opts.values;
    this.operator = opts.operator;
    this.tablePrefix = opts.tablePrefix;
  }

  apply(paramIndex: number): PgFilterResult & { nextParamIndex: number } {
    const fieldWithPrefix = this.tablePrefix
      ? `"${this.tablePrefix}"."${this.field}"`
      : `"${this.field}"`;

    switch (this.operator) {
      case "any of":
        return {
          query: `${fieldWithPrefix} = ANY($${paramIndex}::text[])`,
          params: [this.values],
          nextParamIndex: paramIndex + 1,
        };
      case "none of":
        return {
          query: `NOT (${fieldWithPrefix} = ANY($${paramIndex}::text[]))`,
          params: [this.values],
          nextParamIndex: paramIndex + 1,
        };
      default:
        throw new Error(`Unsupported stringOptions operator: ${this.operator}`);
    }
  }
}

// ============================================================
// FilterList — 组合多个 filter（带数组方法 + apply）
// ============================================================
export class FilterList {
  private filters: Filter[];

  constructor(filters: Filter[] = []) {
    this.filters = filters;
  }

  push(...items: Filter[]): number {
    return this.filters.push(...items);
  }

  find(predicate: (f: Filter) => boolean, thisArg?: unknown): Filter | undefined {
    return this.filters.find(predicate, thisArg);
  }

  filter(predicate: (f: Filter) => boolean, thisArg?: unknown): Filter[] {
    return this.filters.filter(predicate, thisArg);
  }

  some(predicate: (f: Filter) => boolean): boolean {
    return this.filters.some(predicate);
  }

  length(): number {
    return this.filters.length;
  }

  apply(): PgFilterResult {
    return applyFilterList(this.filters);
  }
}

export function applyFilterList(
  filters: Filter[],
): PgFilterResult {
  let paramIndex = 1;
  const clauses: string[] = [];
  const allParams: unknown[] = [];

  for (const filter of filters) {
    const result = filter.apply(paramIndex);
    if (result.query) {
      clauses.push(result.query);
      allParams.push(...result.params);
      paramIndex = result.nextParamIndex;
    }
  }

  return {
    query: clauses.length > 0 ? clauses.join(" AND ") : "1=1",
    params: allParams,
  };
}
