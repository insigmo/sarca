use sqlx::{QueryBuilder, Sqlite};
use uuid::Uuid;

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
