fn main() {
	let path = std::env::args().nth(1).expect("db path required");
	let db = brainstory_lib::db::Db::open(std::path::Path::new(&path)).expect("open db");
	let status = db.get_daily_status();
	println!(
		"streak: {} days | today: completed={} intent={:?} log={:?} survey={:?}",
		status.streak, status.is_completed, status.intent_idea_id, status.log_id, status.survey_id
	);
}
