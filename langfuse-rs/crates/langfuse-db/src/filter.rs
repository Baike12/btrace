use serde::{Deserialize, Serialize};
use sqlx::QueryBuilder;

/// FilterState 对应前端传来的过滤条件数组
/// 每个 FilterCondition 是一个条件表达式
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FilterCondition {
    pub column: String,
    pub operator: FilterOperator,
    pub value: serde_json::Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FilterOperator {
    #[serde(rename = "=")]
    Eq,
    #[serde(rename = "!=")]
    NotEq,
    #[serde(rename = "contains")]
    Contains,
    #[serde(rename = "does not contain")]
    NotContains,
    #[serde(rename = "starts with")]
    StartsWith,
    #[serde(rename = "ends with")]
    EndsWith,
    #[serde(rename = ">")]
    Gt,
    #[serde(rename = ">=")]
    Gte,
    #[serde(rename = "<")]
    Lt,
    #[serde(rename = "<=")]
    Lte,
    #[serde(rename = "any of")]
    AnyOf,
    #[serde(rename = "none of")]
    NoneOf,
    #[serde(rename = "all of")]
    AllOf,
    #[serde(rename = "is null")]
    IsNull,
    #[serde(rename = "is not null")]
    IsNotNull,
}

#[derive(Debug, Clone)]
pub struct FilterResult {
    pub query: String,
    pub params: Vec<serde_json::Value>,
}

/// 将前端 FilterState 翻译为 PG WHERE 子句（追加到已有查询上）。
///
/// # 为什么必须传 `allowed_columns`
///
/// `FilterCondition::column` 来自请求体，而 SQL 里**列名和值不能走同一条通道**：
/// 值可以绑定参数，列名不行。早先的实现直接
/// `query.push(format!(" AND {} = ", filter.column))` 把客户端字符串拼进 SQL，
/// 任何能构造 filter 的调用方都能注入任意 SQL。
///
/// 因此列名只能从**调用方提供的白名单**里取：不在白名单中的条件被跳过并记录
/// warning，而不是拼进语句。这与 `repos::traces::push_trace_filters` 用
/// `TRACE_ORDER_COLUMNS::contains` 校验排序列是同一套做法。
///
/// 目前本函数没有调用方：控制台的过滤走各 repo 里手写的 `push_*_filters`。
/// 保留它是为了让「按 FilterState 动态拼 WHERE」这条路径有一个安全的入口，
/// 而不是留给下一个使用者一个现成的注入点。
pub fn apply_filters(
    query: &mut QueryBuilder<'_, sqlx::Postgres>,
    filters: &[FilterCondition],
    allowed_columns: &[&str],
) {
    for filter in filters {
        if !allowed_columns.contains(&filter.column.as_str()) {
            tracing::warn!(
                column = %filter.column,
                "ignoring filter on a column that is not in the allow-list"
            );
            continue;
        }

        // `is null` / `is not null` carry no value, and `= NULL` is never true
        // in SQL — both need the dedicated operators rather than a bound null.
        match filter.operator {
            FilterOperator::IsNull => {
                query.push(format!(" AND {} IS NULL", filter.column));
                continue;
            }
            FilterOperator::IsNotNull => {
                query.push(format!(" AND {} IS NOT NULL", filter.column));
                continue;
            }
            _ if filter.value.is_null() => {
                tracing::warn!(
                    column = %filter.column,
                    operator = ?filter.operator,
                    "ignoring a filter whose value is null; use `is null` instead"
                );
                continue;
            }
            _ => {}
        }

        match filter.operator {
            FilterOperator::Eq => {
                query.push(format!(" AND {} = ", filter.column));
                push_value(query, &filter.value);
            }
            FilterOperator::NotEq => {
                query.push(format!(" AND {} != ", filter.column));
                push_value(query, &filter.value);
            }
            FilterOperator::Contains => {
                query.push(format!(" AND {} LIKE ", filter.column));
                query.push_bind(like_pattern(filter.value.as_str().unwrap_or_default(), "%", "%"));
            }
            FilterOperator::NotContains => {
                query.push(format!(" AND {} NOT LIKE ", filter.column));
                query.push_bind(like_pattern(filter.value.as_str().unwrap_or_default(), "%", "%"));
            }
            FilterOperator::StartsWith => {
                query.push(format!(" AND {} LIKE ", filter.column));
                query.push_bind(like_pattern(filter.value.as_str().unwrap_or_default(), "", "%"));
            }
            FilterOperator::EndsWith => {
                query.push(format!(" AND {} LIKE ", filter.column));
                query.push_bind(like_pattern(filter.value.as_str().unwrap_or_default(), "%", ""));
            }
            FilterOperator::Gt => {
                query.push(format!(" AND {} > ", filter.column));
                push_value(query, &filter.value);
            }
            FilterOperator::Gte => {
                query.push(format!(" AND {} >= ", filter.column));
                push_value(query, &filter.value);
            }
            FilterOperator::Lt => {
                query.push(format!(" AND {} < ", filter.column));
                push_value(query, &filter.value);
            }
            FilterOperator::Lte => {
                query.push(format!(" AND {} <= ", filter.column));
                push_value(query, &filter.value);
            }
            FilterOperator::AnyOf => {
                if let Some(vals) = string_array(&filter.value) {
                    query.push(format!(" AND {} = ANY(", filter.column));
                    query.push_bind(vals);
                    query.push(")");
                }
            }
            FilterOperator::NoneOf => {
                if let Some(vals) = string_array(&filter.value) {
                    query.push(format!(" AND NOT ({0} = ANY(", filter.column));
                    query.push_bind(vals);
                    query.push("))");
                }
            }
            FilterOperator::AllOf => {
                if let Some(vals) = string_array(&filter.value) {
                    query.push(format!(" AND {} @> ", filter.column));
                    query.push_bind(vals);
                    query.push("::text[]");
                }
            }
            // Handled above; unreachable here, but the match must stay total.
            FilterOperator::IsNull | FilterOperator::IsNotNull => {}
        }
    }
}

/// Build a `LIKE` pattern, escaping the wildcards the *user* typed.
///
/// Without this, a search for `50%` would match everything starting with `50`,
/// and `a_b` would match `axb`. `\` is the default escape character for `LIKE`
/// in PostgreSQL, so escaping it, `%` and `_` is enough.
fn like_pattern(value: &str, prefix: &str, suffix: &str) -> String {
    let escaped = value
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_");
    format!("{prefix}{escaped}{suffix}")
}

fn string_array(value: &serde_json::Value) -> Option<Vec<String>> {
    value
        .as_array()
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
}

fn push_value(query: &mut QueryBuilder<'_, sqlx::Postgres>, value: &serde_json::Value) {
    match value {
        serde_json::Value::String(s) => {
            query.push_bind(s.clone());
        }
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                query.push_bind(i);
            } else if let Some(f) = n.as_f64() {
                query.push_bind(f);
            } else {
                query.push_bind(n.to_string());
            }
        }
        serde_json::Value::Bool(b) => {
            query.push_bind(*b);
        }
        // Nulls are rejected before we get here; binding one would produce
        // `col = $1` with a NULL parameter, which is never true in SQL and
        // silently returns an empty result set.
        serde_json::Value::Null => {
            tracing::warn!("refusing to bind a null value into a comparison");
        }
        _ => {
            query.push_bind(value.to_string());
        }
    }
}

/// OrderBy 排序方向
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderBy {
    pub column: String,
    #[serde(default = "default_direction")]
    pub direction: OrderDirection,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum OrderDirection {
    Asc,
    Desc,
}

fn default_direction() -> OrderDirection {
    OrderDirection::Desc
}

/// 分页信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaginationMeta {
    pub cursor: Option<String>,
    pub has_more: bool,
    pub total: Option<i64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    const COLUMNS: &[&str] = &["name", "user_id"];

    fn build(filters: &[FilterCondition]) -> String {
        let mut qb = sqlx::QueryBuilder::new("SELECT * FROM traces WHERE project_id = 'p'");
        apply_filters(&mut qb, filters, COLUMNS);
        qb.sql().to_string()
    }

    fn cond(column: &str, operator: FilterOperator, value: serde_json::Value) -> FilterCondition {
        FilterCondition {
            column: column.to_string(),
            operator,
            value,
        }
    }

    #[test]
    fn a_column_outside_the_allow_list_is_dropped_entirely() {
        let sql = build(&[cond(
            "name) OR 1=1 --",
            FilterOperator::Eq,
            serde_json::json!("x"),
        )]);
        assert_eq!(sql, "SELECT * FROM traces WHERE project_id = 'p'");
    }

    #[test]
    fn an_injected_column_cannot_smuggle_a_union_into_the_statement() {
        // The exact shape the old `format!(" AND {} = ", column)` allowed.
        let sql = build(&[cond(
            "name = 'x' UNION SELECT password FROM users --",
            FilterOperator::Eq,
            serde_json::json!("x"),
        )]);
        assert!(!sql.to_uppercase().contains("UNION"));
        assert!(!sql.contains("users"));
    }

    #[test]
    fn an_allowed_column_is_still_rendered() {
        let sql = build(&[cond("name", FilterOperator::Eq, serde_json::json!("abc"))]);
        assert!(sql.contains("AND name = $1"), "got: {sql}");
    }

    #[test]
    fn wildcards_typed_by_the_user_are_escaped() {
        let sql = build(&[cond(
            "name",
            FilterOperator::Contains,
            serde_json::json!("50%_x"),
        )]);
        // The bound parameter holds the escaped pattern; assert on the value
        // the caller will send rather than on the SQL text.
        assert_eq!(like_pattern("50%_x", "%", "%"), "%50\\%\\_x%");
        assert!(sql.contains("AND name LIKE $1"), "got: {sql}");
    }

    #[test]
    fn is_null_uses_the_operator_not_a_bound_null() {
        let sql = build(&[cond("name", FilterOperator::IsNull, serde_json::Value::Null)]);
        assert!(sql.contains("AND name IS NULL"), "got: {sql}");
        assert!(!sql.contains("$1"), "must not bind a parameter: {sql}");
    }

    #[test]
    fn a_null_value_on_a_comparison_operator_is_dropped() {
        // `= NULL` is never true, so the condition would silently empty the
        // result set instead of erroring.
        let sql = build(&[cond("name", FilterOperator::Eq, serde_json::Value::Null)]);
        assert_eq!(sql, "SELECT * FROM traces WHERE project_id = 'p'");
    }

    #[test]
    fn array_operators_bind_one_parameter_for_the_whole_list() {
        let sql = build(&[cond(
            "user_id",
            FilterOperator::AnyOf,
            serde_json::json!(["a", "b"]),
        )]);
        assert!(sql.contains("AND user_id = ANY($1)"), "got: {sql}");
    }
}
