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

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS vote_snapshots (
            id           INTEGER PRIMARY KEY AUTOINCREMENT,
            candidate_id TEXT NOT NULL,
            vote_count   INTEGER NOT NULL,
            captured_at  INTEGER NOT NULL DEFAULT (unixepoch())
        )",
    )
    .execute(pool)
    .await?;

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_snapshots_candidate_date ON vote_snapshots(candidate_id, captured_at)",
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

/// Enregistre un instantané des compteurs actuels (un point par candidat, même timestamp).
pub async fn insert_snapshot(
    pool: &SqlitePool,
    counts: &DashMap<String, i64>,
) -> Result<(), sqlx::Error> {
    if counts.is_empty() {
        return Ok(());
    }

    let captured_at = chrono::Utc::now().timestamp();
    let mut tx = pool.begin().await?;

    for entry in counts.iter() {
        sqlx::query(
            "INSERT INTO vote_snapshots (candidate_id, vote_count, captured_at) VALUES (?, ?, ?)",
        )
        .bind(entry.key())
        .bind(*entry.value())
        .bind(captured_at)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;
    Ok(())
}

/// Historique des voix dans le temps pour les candidats demandés (tous si `candidate_ids` est vide).
pub async fn get_history(
    pool: &SqlitePool,
    candidate_ids: &[String],
) -> Result<Vec<(String, i64, i64)>, sqlx::Error> {
    if candidate_ids.is_empty() {
        sqlx::query_as(
            "SELECT candidate_id, vote_count, captured_at FROM vote_snapshots ORDER BY captured_at ASC",
        )
        .fetch_all(pool)
        .await
    } else {
        let placeholders = candidate_ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let sql = format!(
            "SELECT candidate_id, vote_count, captured_at FROM vote_snapshots WHERE candidate_id IN ({}) ORDER BY captured_at ASC",
            placeholders
        );
        let mut query = sqlx::query_as(&sql);
        for id in candidate_ids {
            query = query.bind(id);
        }
        query.fetch_all(pool).await
    }
}
