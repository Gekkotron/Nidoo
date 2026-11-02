//! The turn log in the history database (table `assist_turn`).

use nidoo_assist::view::{AssistTurn, OutcomeKind};
use sqlx::Row;

use crate::history::db::Db;

fn secs(v: u64) -> i64 {
    i64::try_from(v).unwrap_or(i64::MAX)
}

impl Db {
    /// Logs one turn; a turn already logged under its id is kept as is.
    pub async fn insert_assist_turn(&self, t: &AssistTurn) -> Result<(), sqlx::Error> {
        let question = t
            .question
            .as_ref()
            .map(|q| serde_json::to_value(q).unwrap_or_default());
        let trace = serde_json::to_value(&t.trace).unwrap_or_default();
        sqlx::query(
            "INSERT INTO assist_turn
                 (id, at, client, text, simulate, outcome, reply, question, trace, wrong)
             VALUES ($1, to_timestamp($2::double precision), $3, $4, $5, $6, $7, $8, $9, $10)
             ON CONFLICT (id) DO NOTHING",
        )
        .bind(&t.id)
        .bind(secs(t.at))
        .bind(&t.client)
        .bind(&t.text)
        .bind(t.simulate)
        .bind(t.outcome.as_str())
        .bind(&t.reply)
        .bind(question)
        .bind(trace)
        .bind(t.wrong)
        .execute(self.pool())
        .await?;
        Ok(())
    }

    /// Up to `limit` turns, newest first.
    pub async fn assist_turns(&self, limit: i64) -> Result<Vec<AssistTurn>, sqlx::Error> {
        let rows = sqlx::query(
            "SELECT id, extract(epoch FROM at)::bigint, client, text, simulate, outcome, reply,
                    question, trace, wrong
             FROM assist_turn
             ORDER BY at DESC, seq DESC
             LIMIT $1",
        )
        .bind(limit.max(0))
        .fetch_all(self.pool())
        .await?;
        Ok(rows
            .iter()
            .map(|r| AssistTurn {
                id: r.get(0),
                at: u64::try_from(r.get::<i64, _>(1)).unwrap_or(0),
                client: r.get(2),
                text: r.get(3),
                simulate: r.get(4),
                outcome: OutcomeKind::parse(r.get::<&str, _>(5)).unwrap_or(OutcomeKind::Failed),
                reply: r.get(6),
                question: r
                    .get::<Option<serde_json::Value>, _>(7)
                    .and_then(|v| serde_json::from_value(v).ok()),
                trace: serde_json::from_value(r.get(8)).unwrap_or_default(),
                wrong: r.get(9),
            })
            .collect())
    }

    /// Flags a turn as wrongly understood; false when no turn has that id.
    pub async fn mark_assist_wrong(&self, id: &str) -> Result<bool, sqlx::Error> {
        let done = sqlx::query("UPDATE assist_turn SET wrong = true WHERE id = $1")
            .bind(id)
            .execute(self.pool())
            .await?;
        Ok(done.rows_affected() > 0)
    }

    /// Deletes the turns older than `before` (Unix seconds); returns how many.
    pub async fn purge_assist(&self, before: u64) -> Result<u64, sqlx::Error> {
        let done =
            sqlx::query("DELETE FROM assist_turn WHERE at < to_timestamp($1::double precision)")
                .bind(secs(before))
                .execute(self.pool())
                .await?;
        Ok(done.rows_affected())
    }
}
