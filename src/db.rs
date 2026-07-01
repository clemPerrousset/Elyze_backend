use dashmap::DashMap;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::SqlitePool;
use std::str::FromStr;

use crate::state::PendingOp;

pub async fn create_pool(database_url: &str) -> Result<SqlitePool, sqlx::Error> {
    let options = SqliteConnectOptions::from_str(database_url)?
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .foreign_keys(true);

    SqlitePoolOptions::new()
        .max_connections(4)
        .connect_with(options)
        .await
}

pub async fn run_migrations(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS candidates (
            id TEXT PRIMARY KEY,
            created_at INTEGER NOT NULL DEFAULT (unixepoch())
        )",
    )
    .execute(pool)
    .await?;

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS votes (
            phone_id     TEXT PRIMARY KEY,
            candidate_id TEXT NOT NULL,
            voted_at     INTEGER NOT NULL DEFAULT (unixepoch()),
            FOREIGN KEY (candidate_id) REFERENCES candidates(id) ON DELETE CASCADE
        )",
    )
    .execute(pool)
    .await?;

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_votes_candidate ON votes(candidate_id)",
    )
    .execute(pool)
    .await?;

    Ok(())
}

/// Charge tous les votes depuis SQLite au démarrage.
pub async fn load_votes(pool: &SqlitePool) -> Result<DashMap<String, String>, sqlx::Error> {
    let map = DashMap::new();
    let rows: Vec<(String, String)> =
        sqlx::query_as("SELECT phone_id, candidate_id FROM votes")
            .fetch_all(pool)
            .await?;
    for (phone_id, candidate_id) in rows {
        map.insert(phone_id, candidate_id);
    }
    Ok(map)
}

/// Charge les compteurs depuis SQLite au démarrage.
pub async fn load_counts(pool: &SqlitePool) -> Result<DashMap<String, i64>, sqlx::Error> {
    let map = DashMap::new();

    let candidates: Vec<(String,)> =
        sqlx::query_as("SELECT id FROM candidates")
            .fetch_all(pool)
            .await?;
    for (id,) in candidates {
        map.insert(id, 0i64);
    }

    let counts: Vec<(String, i64)> =
        sqlx::query_as("SELECT candidate_id, COUNT(*) FROM votes GROUP BY candidate_id")
            .fetch_all(pool)
            .await?;
    for (id, cnt) in counts {
        map.insert(id, cnt);
    }

    Ok(map)
}

/// Flush un batch d'opérations en SQLite dans une seule transaction.
pub async fn flush(pool: &SqlitePool, ops: Vec<PendingOp>) -> Result<(), sqlx::Error> {
    if ops.is_empty() {
        return Ok(());
    }

    let mut tx = pool.begin().await?;

    for op in ops {
        match op {
            PendingOp::Upsert(phone_id, candidate_id) => {
                sqlx::query(
                    "INSERT INTO candidates (id) VALUES (?) ON CONFLICT DO NOTHING",
                )
                .bind(&candidate_id)
                .execute(&mut *tx)
                .await?;

                sqlx::query(
                    "INSERT INTO votes (phone_id, candidate_id) VALUES (?, ?)
                     ON CONFLICT(phone_id) DO UPDATE SET candidate_id = excluded.candidate_id, voted_at = unixepoch()",
                )
                .bind(&phone_id)
                .bind(&candidate_id)
                .execute(&mut *tx)
                .await?;
            }
            PendingOp::Delete(phone_id) => {
                sqlx::query("DELETE FROM votes WHERE phone_id = ?")
                    .bind(&phone_id)
                    .execute(&mut *tx)
                    .await?;
            }
        }
    }

    tx.commit().await?;
    Ok(())
}
