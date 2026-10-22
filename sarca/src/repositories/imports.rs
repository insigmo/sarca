use chrono::{DateTime, Utc};
use sqlx::{QueryBuilder, SqlitePool};
use uuid::Uuid;

use crate::{
    errors::{SarcaError, SarcaResult},
    models::imports::{
        ITEM_FAILED,
        ITEM_IN_PROGRESS,
        ITEM_PENDING,
        ImportItem,
        ImportJob,
        ImportJobSummary,
        JOB_RUNNING,
        JOB_SCANNING,
    },
};

/// A file found on the source, before it has an id.
#[derive(Debug, Clone)]
pub struct NewImportItem {
    pub rel_path: String,
    pub size: i64,
    pub source_mtime: Option<DateTime<Utc>>,
}

/// What a new job is created from.
#[derive(Debug, Clone)]
pub struct NewImportJob<'a> {
    pub user_id: Uuid,
    pub storage_id: Uuid,
    pub source_kind: &'a str,
    pub source_url: &'a str,
    pub username: &'a str,
    pub secret: &'a str,
    pub target_path: &'a str,
}

pub struct ImportsRepository<'d> {
    db: &'d SqlitePool,
}

fn db_error(context: &str, e: &sqlx::Error) -> SarcaError {
    tracing::error!("imports {context}: {e}");
    SarcaError::Unknown
}

/// Job columns plus item counts, so the list and the single-job read agree.
const SUMMARY_SELECT: &str = r#"
    SELECT j.id, j.storage_id, j.source_kind, j.source_url, j.username,
           j.secret IS NOT NULL AS has_secret,
           j.target_path, j.status, j.last_error, j.created_at, j.updated_at, j.completed_at,
           COUNT(i.id) AS total_files,
           COALESCE(SUM(i.size), 0) AS total_bytes,
           COALESCE(SUM(i.status = 'done'), 0) AS done_files,
           COALESCE(SUM(CASE WHEN i.status IN ('done', 'skipped') THEN i.size ELSE 0 END), 0)
             AS done_bytes,
           COALESCE(SUM(i.status = 'skipped'), 0) AS skipped_files,
           COALESCE(SUM(i.status = 'failed'), 0) AS failed_files
      FROM import_jobs j
      LEFT JOIN import_items i ON i.job_id = j.id
"#;

impl<'d> ImportsRepository<'d> {
    pub fn new(db: &'d SqlitePool) -> Self {
        Self {
            db,
        }
    }

    pub async fn create_job(&self, job: NewImportJob<'_>) -> SarcaResult<Uuid> {
        let id = Uuid::new_v4();
        sqlx::query(
            r#"
            INSERT INTO import_jobs
                (id, user_id, storage_id, source_kind, source_url, username, secret, target_path)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            "#,
        )
        .bind(id)
        .bind(job.user_id)
        .bind(job.storage_id)
        .bind(job.source_kind)
        .bind(job.source_url)
        .bind(job.username)
        .bind(job.secret)
        .bind(job.target_path)
        .execute(self.db)
        .await
        .map_err(|e| db_error("create job", &e))?;
        Ok(id)
    }

    pub async fn list_summaries(&self) -> SarcaResult<Vec<ImportJobSummary>> {
        sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "{SUMMARY_SELECT} GROUP BY j.id ORDER BY j.created_at DESC"
        )))
        .fetch_all(self.db)
        .await
        .map_err(|e| db_error("list jobs", &e))
    }

    pub async fn get_summary(&self, id: Uuid) -> SarcaResult<ImportJobSummary> {
        sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "{SUMMARY_SELECT} WHERE j.id = $1 GROUP BY j.id"
        )))
        .bind(id)
        .fetch_optional(self.db)
        .await
        .map_err(|e| db_error("get job", &e))?
        .ok_or_else(|| SarcaError::DoesNotExist("import".to_owned()))
    }

    pub async fn get_job(&self, id: Uuid) -> SarcaResult<Option<ImportJob>> {
        sqlx::query_as("SELECT * FROM import_jobs WHERE id = $1")
            .bind(id)
            .fetch_optional(self.db)
            .await
            .map_err(|e| db_error("get job", &e))
    }

    /// The job the runner should work on next: the oldest one still active.
    pub async fn next_active_job(&self) -> SarcaResult<Option<ImportJob>> {
        sqlx::query_as(
            r#"
            SELECT * FROM import_jobs
             WHERE status IN ($1, $2)
             ORDER BY created_at, id
             LIMIT 1
            "#,
        )
        .bind(JOB_SCANNING)
        .bind(JOB_RUNNING)
        .fetch_optional(self.db)
        .await
        .map_err(|e| db_error("next job", &e))
    }

    /// Move a job to `to`, but only from one of `from` — so a cancel that lands
    /// while the runner finishes a scan is not overwritten with `running`.
    pub async fn transition(
        &self,
        id: Uuid,
        from: &[&str],
        to: &str,
        last_error: Option<&str>,
    ) -> SarcaResult<bool> {
        let mut q = QueryBuilder::new("UPDATE import_jobs SET status = ");
        q.push_bind(to);
        q.push(", last_error = ");
        q.push_bind(last_error);
        q.push(", updated_at = datetime('now')");
        q.push(", completed_at = CASE WHEN ");
        q.push_bind(to);
        q.push(" IN ('done', 'failed', 'cancelled') THEN datetime('now') ELSE NULL END");
        q.push(" WHERE id = ");
        q.push_bind(id);
        q.push(" AND status IN (");
        let mut sep = q.separated(", ");
        for status in from {
            sep.push_bind(*status);
        }
        q.push(")");
        let done = q.build().execute(self.db).await.map_err(|e| db_error("transition", &e))?;
        Ok(done.rows_affected() > 0)
    }

    pub async fn set_secret(&self, id: Uuid, secret: &str) -> SarcaResult<()> {
        sqlx::query(
            "UPDATE import_jobs SET secret = $2, updated_at = datetime('now') WHERE id = $1",
        )
        .bind(id)
        .bind(secret)
        .execute(self.db)
        .await
        .map_err(|e| db_error("set secret", &e))?;
        Ok(())
    }

    pub async fn delete_job(&self, id: Uuid) -> SarcaResult<bool> {
        // Items go explicitly too: the cascade only fires with `foreign_keys`
        // on, and a connection opened elsewhere (a test, a restore) may not be.
        sqlx::query("DELETE FROM import_items WHERE job_id = $1")
            .bind(id)
            .execute(self.db)
            .await
            .map_err(|e| db_error("delete items", &e))?;
        let done = sqlx::query("DELETE FROM import_jobs WHERE id = $1")
            .bind(id)
            .execute(self.db)
            .await
            .map_err(|e| db_error("delete job", &e))?;
        Ok(done.rows_affected() > 0)
    }

    /// Record files found by a scan. A rescan after a restart finds the same
    /// paths again, and the unique key keeps those as they were.
    pub async fn insert_items(&self, job_id: Uuid, items: &[NewImportItem]) -> SarcaResult<()> {
        // SQLite caps bound parameters per statement; 4 per row stays far under it.
        for batch in items.chunks(200) {
            QueryBuilder::new(
                "INSERT OR IGNORE INTO import_items (job_id, rel_path, size, source_mtime) ",
            )
            .push_values(batch, |mut q, item| {
                q.push_bind(job_id)
                    .push_bind(&item.rel_path)
                    .push_bind(item.size)
                    .push_bind(item.source_mtime);
            })
            .build()
            .execute(self.db)
            .await
            .map_err(|e| db_error("insert items", &e))?;
        }
        Ok(())
    }

    /// Take the next pending item of `job_id`, fewest attempts first so one file
    /// that keeps failing does not hold the rest back.
    pub async fn claim_next_item(&self, job_id: Uuid) -> SarcaResult<Option<ImportItem>> {
        sqlx::query_as(
            r#"
            UPDATE import_items
               SET status = $2, attempts = attempts + 1, updated_at = datetime('now')
             WHERE id = (
                   SELECT id FROM import_items
                    WHERE job_id = $1 AND status = $3
                    ORDER BY attempts, id
                    LIMIT 1)
            RETURNING *
            "#,
        )
        .bind(job_id)
        .bind(ITEM_IN_PROGRESS)
        .bind(ITEM_PENDING)
        .fetch_optional(self.db)
        .await
        .map_err(|e| db_error("claim item", &e))
    }

    pub async fn finish_item(
        &self,
        id: i64,
        status: &str,
        last_error: Option<&str>,
    ) -> SarcaResult<()> {
        sqlx::query(
            r#"
            UPDATE import_items
               SET status = $2, last_error = $3, updated_at = datetime('now')
             WHERE id = $1
            "#,
        )
        .bind(id)
        .bind(status)
        .bind(last_error)
        .execute(self.db)
        .await
        .map_err(|e| db_error("finish item", &e))?;
        Ok(())
    }

    /// Put failed items back in the queue with a fresh attempt count.
    pub async fn retry_failed(&self, job_id: Uuid) -> SarcaResult<u64> {
        let done = sqlx::query(
            r#"
            UPDATE import_items
               SET status = $2, attempts = 0, last_error = NULL, updated_at = datetime('now')
             WHERE job_id = $1 AND status = $3
            "#,
        )
        .bind(job_id)
        .bind(ITEM_PENDING)
        .bind(ITEM_FAILED)
        .execute(self.db)
        .await
        .map_err(|e| db_error("retry failed", &e))?;
        Ok(done.rows_affected())
    }

    pub async fn list_failed(&self, job_id: Uuid, limit: i64) -> SarcaResult<Vec<ImportItem>> {
        sqlx::query_as(
            "SELECT * FROM import_items WHERE job_id = $1 AND status = $2 ORDER BY id LIMIT $3",
        )
        .bind(job_id)
        .bind(ITEM_FAILED)
        .bind(limit)
        .fetch_all(self.db)
        .await
        .map_err(|e| db_error("list failed", &e))
    }

    /// Items a process that died mid-copy left claimed. Nothing in a fresh
    /// process is copying them, so they go back in the queue; the upload path
    /// resumes whatever chunks the last attempt stored.
    pub async fn requeue_in_progress(&self) -> SarcaResult<u64> {
        let done = sqlx::query("UPDATE import_items SET status = $1 WHERE status = $2")
            .bind(ITEM_PENDING)
            .bind(ITEM_IN_PROGRESS)
            .execute(self.db)
            .await
            .map_err(|e| db_error("requeue", &e))?;
        Ok(done.rows_affected())
    }

    /// Whether the job has nothing left to do.
    pub async fn is_drained(&self, job_id: Uuid) -> SarcaResult<bool> {
        let left: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM import_items WHERE job_id = $1 AND status IN ($2, $3)",
        )
        .bind(job_id)
        .bind(ITEM_PENDING)
        .bind(ITEM_IN_PROGRESS)
        .fetch_one(self.db)
        .await
        .map_err(|e| db_error("drained", &e))?;
        Ok(left == 0)
    }

    pub async fn count_failed(&self, job_id: Uuid) -> SarcaResult<i64> {
        sqlx::query_scalar("SELECT COUNT(*) FROM import_items WHERE job_id = $1 AND status = $2")
            .bind(job_id)
            .bind(ITEM_FAILED)
            .fetch_one(self.db)
            .await
            .map_err(|e| db_error("count failed", &e))
    }

    /// The listing finished: move to copying, unless the job was paused or
    /// cancelled while it ran.
    pub async fn finish_scan(&self, id: Uuid) -> SarcaResult<bool> {
        let done = sqlx::query(
            r#"
            UPDATE import_jobs
               SET scanned = 1, status = $2, updated_at = datetime('now')
             WHERE id = $1 AND status = $3
            "#,
        )
        .bind(id)
        .bind(JOB_RUNNING)
        .bind(JOB_SCANNING)
        .execute(self.db)
        .await
        .map_err(|e| db_error("finish scan", &e))?;
        Ok(done.rows_affected() > 0)
    }

    /// Hand a claimed item back untouched — the job stopped before it was tried,
    /// so the claim does not count as an attempt.
    pub async fn release_item(&self, id: i64) -> SarcaResult<()> {
        sqlx::query(
            r#"
            UPDATE import_items
               SET status = $2, attempts = MAX(attempts - 1, 0), updated_at = datetime('now')
             WHERE id = $1
            "#,
        )
        .bind(id)
        .bind(ITEM_PENDING)
        .execute(self.db)
        .await
        .map_err(|e| db_error("release item", &e))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::imports::{ITEM_DONE, ITEM_SKIPPED, JOB_DONE, JOB_PAUSED};

    async fn test_pool() -> SqlitePool {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sarca.sqlite");
        let pool = crate::common::db::pool::get_pool(
            path.to_str().unwrap(),
            4,
            std::time::Duration::from_secs(5),
        )
        .await
        .unwrap();
        crate::startup::init_db(&pool).await;
        std::mem::forget(dir);
        pool
    }

    async fn job(pool: &SqlitePool) -> Uuid {
        let user_id = Uuid::new_v4();
        sqlx::query("INSERT INTO users (id, email) VALUES ($1, 'a@example.com')")
            .bind(user_id)
            .execute(pool)
            .await
            .unwrap();
        let storage_id = Uuid::new_v4();
        sqlx::query("INSERT INTO storages (id, name, primary_position) VALUES ($1, 's', 1)")
            .bind(storage_id)
            .execute(pool)
            .await
            .unwrap();
        ImportsRepository::new(pool)
            .create_job(NewImportJob {
                user_id,
                storage_id,
                source_kind: "webdav",
                source_url: "https://cloud.example.com/dav/",
                username: "alice",
                secret: "app-password",
                target_path: "Imported",
            })
            .await
            .unwrap()
    }

    fn item(path: &str, size: i64) -> NewImportItem {
        NewImportItem {
            rel_path: path.to_owned(),
            size,
            source_mtime: None,
        }
    }

    #[tokio::test]
    async fn a_rescan_does_not_double_items_and_counts_add_up() {
        let pool = test_pool().await;
        let id = job(&pool).await;
        let repo = ImportsRepository::new(&pool);

        repo.insert_items(id, &[item("a.jpg", 10), item("b/c.mp4", 30)]).await.unwrap();
        repo.insert_items(id, &[item("a.jpg", 10), item("d.txt", 5)]).await.unwrap();

        let first = repo.claim_next_item(id).await.unwrap().unwrap();
        assert_eq!(first.rel_path, "a.jpg");
        assert_eq!(first.attempts, 1);
        repo.finish_item(first.id, ITEM_DONE, None).await.unwrap();
        let second = repo.claim_next_item(id).await.unwrap().unwrap();
        repo.finish_item(second.id, ITEM_SKIPPED, None).await.unwrap();
        let third = repo.claim_next_item(id).await.unwrap().unwrap();
        repo.finish_item(third.id, ITEM_FAILED, Some("boom")).await.unwrap();

        let summary = repo.get_summary(id).await.unwrap();
        assert_eq!(summary.total_files, 3);
        assert_eq!(summary.total_bytes, 45);
        assert_eq!(summary.done_files, 1);
        assert_eq!(summary.skipped_files, 1);
        assert_eq!(summary.failed_files, 1);
        assert_eq!(summary.done_bytes, 10 + second.size);
        assert!(summary.has_secret);
        assert!(repo.is_drained(id).await.unwrap());

        assert_eq!(repo.retry_failed(id).await.unwrap(), 1);
        assert!(!repo.is_drained(id).await.unwrap());
        let retried = repo.claim_next_item(id).await.unwrap().unwrap();
        assert_eq!((retried.rel_path.as_str(), retried.attempts), (third.rel_path.as_str(), 1));
    }

    #[tokio::test]
    async fn released_and_interrupted_items_go_back_without_losing_attempts() {
        let pool = test_pool().await;
        let id = job(&pool).await;
        let repo = ImportsRepository::new(&pool);
        repo.insert_items(id, &[item("a", 1), item("b", 1)]).await.unwrap();

        let a = repo.claim_next_item(id).await.unwrap().unwrap();
        repo.release_item(a.id).await.unwrap();
        let again = repo.claim_next_item(id).await.unwrap().unwrap();
        assert_eq!((again.id, again.attempts), (a.id, 1));

        // A crash leaves it claimed; the next process puts it back.
        assert_eq!(repo.requeue_in_progress().await.unwrap(), 1);
        assert!(repo.claim_next_item(id).await.unwrap().is_some());
    }

    #[tokio::test]
    async fn transitions_only_apply_from_the_listed_states() {
        let pool = test_pool().await;
        let id = job(&pool).await;
        let repo = ImportsRepository::new(&pool);

        assert!(!repo.transition(id, &[JOB_RUNNING], JOB_PAUSED, None).await.unwrap());
        assert!(repo.transition(id, &[JOB_SCANNING], JOB_PAUSED, None).await.unwrap());
        // A pause that lands mid-listing wins over the scan finishing.
        assert!(!repo.finish_scan(id).await.unwrap());
        let job = repo.get_job(id).await.unwrap().unwrap();
        assert_eq!(job.status, JOB_PAUSED);
        assert!(!job.scanned);
        assert!(job.completed_at.is_none());

        assert!(repo.transition(id, &[JOB_PAUSED], JOB_SCANNING, None).await.unwrap());
        assert!(repo.finish_scan(id).await.unwrap());
        assert!(repo.transition(id, &[JOB_RUNNING], JOB_DONE, None).await.unwrap());
        let job = repo.get_job(id).await.unwrap().unwrap();
        assert!(job.scanned);
        assert!(job.completed_at.is_some());
        assert_eq!(repo.next_active_job().await.unwrap().map(|j| j.id), None);
    }

    #[tokio::test]
    async fn deleting_a_job_removes_its_items() {
        let pool = test_pool().await;
        let id = job(&pool).await;
        let repo = ImportsRepository::new(&pool);
        repo.insert_items(id, &[item("a", 1)]).await.unwrap();
        assert!(repo.delete_job(id).await.unwrap());
        let left: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM import_items").fetch_one(&pool).await.unwrap();
        assert_eq!(left, 0);
    }
}
