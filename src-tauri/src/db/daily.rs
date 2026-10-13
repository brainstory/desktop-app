//! The daily loop's storage: logs, surveys, daily status and the
//! streak walk.

use chrono::NaiveDate;
use rusqlite::{params, Connection, OptionalExtension};

use super::{now_iso, own_activity_filter, Db};
use crate::types::DailyStatus;

impl Db {
	pub fn insert_log(
		&self,
		id: &str,
		answers: &[crate::types::LogAnswerItem],
	) -> Result<(), String> {
		let json = serde_json::to_string(answers).unwrap_or_else(|_| "[]".into());
		let today = self.today_local().format("%Y-%m-%d").to_string();
		self.with_tx(|conn| {
			conn.execute(
				"INSERT INTO log_entries (id, answers, created_at, local_date) VALUES (?1, ?2, ?3, ?4)",
				params![id, json, now_iso(), today],
			)
			.map_err(|e| format!("failed to save daily log: {e}"))?;
			conn.execute(
				"INSERT INTO daily (date, log_id, is_completed, created_at) VALUES (?1, ?2, 0, ?3)
				 ON CONFLICT(date) DO UPDATE SET log_id = ?2",
				params![today, id, now_iso()],
			)
			.map_err(|e| format!("failed to save daily log: {e}"))?;
			Ok(())
		})
	}

	pub fn get_log_answers_today(&self) -> Option<Vec<crate::types::LogAnswerItem>> {
		let today = self.today_local().format("%Y-%m-%d").to_string();
		let conn = self.lock();
		// a read failure is logged before degrading to "no log today": the
		// caller's None cannot tell the two apart, so the log is the only
		// trace of a broken store
		let log_id: Option<String> = match conn
			.query_row(
				"SELECT log_id FROM daily WHERE date = ?1",
				params![today],
				|row| row.get(0),
			)
			.optional()
		{
			Ok(value) => value.flatten(),
			Err(e) => {
				log::warn!("today's daily log lookup failed: {e}");
				None
			}
		};
		let log_id = log_id?;
		let answers: String = conn
			.query_row(
				"SELECT answers FROM log_entries WHERE id = ?1",
				params![log_id],
				|row| row.get(0),
			)
			.optional()
			.ok()
			.flatten()?;
		serde_json::from_str(&answers).ok()
	}

	pub fn insert_survey(
		&self,
		id: &str,
		idea_id: Option<&str>,
		answers: &serde_json::Value,
	) -> Result<(), String> {
		let today = self.today_local().format("%Y-%m-%d").to_string();
		self.with_tx(|conn| {
			conn.execute(
				"INSERT INTO surveys (id, idea_id, answers, created_at, local_date) VALUES (?1, ?2, ?3, ?4, ?5)",
				params![id, idea_id, answers.to_string(), now_iso(), today],
			)
			.map_err(|e| format!("failed to save survey: {e}"))?;
			// Note: a survey does NOT complete the day. is_completed means
			// "today's daily intent was finished" (the dashboard's intent
			// status card reads it); reflecting on the day is recorded via
			// survey_id and counts toward the streak on its own.
			conn.execute(
				"INSERT INTO daily (date, survey_id, created_at) VALUES (?1, ?2, ?3)
				 ON CONFLICT(date) DO UPDATE SET survey_id = ?2",
				params![today, id, now_iso()],
			)
			.map_err(|e| format!("failed to save survey: {e}"))?;
			Ok(())
		})
	}

	/// Mark today's daily row complete when `idea_id` is its intent.
	/// Ideas without a (today's) daily association match zero rows; the
	/// day/timezone semantics stay exactly as they were.
	pub(super) fn complete_daily_tx(
		conn: &Connection,
		idea_id: &str,
		today: NaiveDate,
	) -> Result<(), String> {
		conn.execute(
			"UPDATE daily SET is_completed = 1 WHERE date = ?1 AND intent_idea_id = ?2",
			params![today.format("%Y-%m-%d").to_string(), idea_id],
		)
		.map_err(|e| format!("failed to complete daily intent: {e}"))?;
		Ok(())
	}

	pub fn mark_daily_completed(&self, idea_id: &str) -> Result<(), String> {
		let today = self.today_local();
		self.with_tx(|conn| Self::complete_daily_tx(conn, idea_id, today))
	}

	/// True if any idea, log or survey was recorded today (local time) -
	/// used to skip the daily reminder on already-active days.
	pub fn has_activity_today(&self) -> bool {
		let today = self.today_local().format("%Y-%m-%d").to_string();
		let conn = self.lock();
		for table in ["ideas", "log_entries", "surveys"] {
			// Imports are filtered explicitly, not only through their empty
			// local_date: older builds let the open-time backfill stamp a
			// date onto them.
			let sql = format!(
				"SELECT EXISTS(SELECT 1 FROM {table} WHERE local_date = ?1{})",
				own_activity_filter(table)
			);
			if let Ok(1) = conn.query_row(&sql, params![today], |row| row.get::<_, i64>(0)) {
				return true;
			}
		}
		false
	}

	pub fn get_daily_status(&self) -> DailyStatus {
		let today = self.today_local().format("%Y-%m-%d").to_string();
		let conn = self.lock();
		let row = conn
			.query_row(
				"SELECT log_id, intent_idea_id, survey_id, is_completed FROM daily WHERE date = ?1",
				params![today],
				|row| {
					Ok((
						row.get::<_, Option<String>>(0)?,
						row.get::<_, Option<String>>(1)?,
						row.get::<_, Option<String>>(2)?,
						row.get::<_, i64>(3)? != 0,
					))
				},
			)
			.optional()
			.ok()
			.flatten();
		let (log_id, intent_idea_id, survey_id, is_completed) =
			row.unwrap_or((None, None, None, false));

		// Streak = consecutive days the daily intent loop was worked on: a
		// day counts when a daily log was submitted (logs only come from
		// the daily intent flow), a daily intent was started, or one was
		// completed. Ordinary ideas and the retired survey flow don't feed
		// it, so the dashboard card's copy ("Do today's daily intent")
		// states exactly what keeps the streak alive. Each activity's
		// local calendar day was frozen at write time (local_date), so the
		// history is stable across timezone changes.
		//
		// The walk is anchored at today: one indexed existence check per
		// day counted, instead of loading every activity date in history
		// on each dashboard load. Cost no longer grows with the library.
		let day_counted = |day: NaiveDate| -> bool {
			let date = day.format("%Y-%m-%d").to_string();
			let logged = conn
				.query_row(
					"SELECT EXISTS(SELECT 1 FROM log_entries WHERE local_date = ?1)",
					params![date],
					|row| row.get::<_, i64>(0),
				)
				.map(|v| v != 0)
				.unwrap_or(false);
			if logged {
				return true;
			}
			conn.query_row(
				"SELECT EXISTS(SELECT 1 FROM daily WHERE date = ?1 AND (intent_idea_id IS NOT NULL OR is_completed = 1))",
				params![date],
				|row| row.get::<_, i64>(0),
			)
			.map(|v| v != 0)
			.unwrap_or(false)
		};

		// Walk back from today; if today has no activity yet the streak
		// isn't broken (it continues from yesterday).
		let today_date = self.today_local();
		let mut streak = 0i64;
		let mut day = if day_counted(today_date) {
			Some(today_date)
		} else {
			today_date.pred_opt()
		};
		while let Some(d) = day {
			if !day_counted(d) {
				break;
			}
			streak += 1;
			day = d.pred_opt();
		}

		DailyStatus {
			log_id,
			intent_idea_id,
			survey_id,
			is_completed,
			streak,
		}
	}
}
