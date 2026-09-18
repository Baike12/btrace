// Stub for PG-only fork: FTS (full-text search) not supported on PG observations table
// Returning false/no-match defaults for all FTS checks
export const FTS_EVENTS_TABLES: string[] = [];
export const FTS_MATCH_OPERATOR = "";
export const FTS_METADATA_FIELD = "";
export const FTS_TEXT_FIELDS: string[] = [];
export const FTS_TEXT_OPERATORS: string[] = [];

export function bareFtsField(_field: string): string { return ""; }
export function hasFtsSearchToken(_query: string): boolean { return false; }
export function isFtsAcceleratedIoOperator(_operator: string): boolean { return false; }
export function isFtsEventsTable(_table: string): boolean { return false; }
export function isFtsMatchOperator(_operator: string): boolean { return false; }
export function isFtsMetadataField(_field: string): boolean { return false; }
export function isFtsMetadataTarget(_field: string): boolean { return false; }
export function isFtsTextField(_field: string): boolean { return false; }
export function isFtsTextTarget(_field: string): boolean { return false; }
