use sqlx::{QueryBuilder, Sqlite};
use uuid::Uuid;

/// SQL condition: `column` starts with the text bound at `param` (or equals it).
///
/// Written as a range rather than `column LIKE param || '%'`, because `SQLite`
/// cannot use an index for a `LIKE` whose pattern is an expression: every folder
/// listing, move and delete scanned the whole `files` table. The range is
/// answered straight from the `path` index.
///
/// It is also exact where `LIKE` was not: case-sensitive like every other path
/// comparison here, and a `_` or `%` in a folder name is just a character.
///
/// The upper bound appends U+10FFFF, the highest code point: under `BINARY`
/// collation every path that continues the prefix sorts below it.
pub fn starts_with(column: &str, param: &str) -> String {
    format!("({column} >= {param} AND {column} < {param} || char(1114111))")
}

/// Push a comma separated list of UUID binds for an `IN (…)` clause.
///
/// `SQLite` cannot bind an array to a single placeholder the way `Postgres` does
/// with `= ANY($1)`, so each id needs its own bind.
pub fn push_uuid_list(builder: &mut QueryBuilder<'_, Sqlite>, ids: &[Uuid]) {
    let mut separated = builder.separated(", ");
    for id in ids {
        separated.push_bind(*id);
    }
}
