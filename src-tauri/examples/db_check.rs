//! Quick DB sanity check: open a brainstory.db and read ideas.
//! Usage: cargo run --release --example db_check -- /path/to/brainstory.db

use brainstory_lib::db::Db;

fn main() {
	let path = std::env::args().nth(1).expect("db path required");
	let db = Db::open(std::path::Path::new(&path)).expect("open db");

	let ideas = db.list_ideas();
	println!("list_ideas: {} ideas", ideas.len());
	for idea in &ideas {
		println!(
			"  {} type={} title={:?} transcript_msgs={} creator={:?}",
			idea.id,
			idea.r#type.as_deref().unwrap_or("?"),
			idea.title,
			idea.transcript.as_ref().map(|t| t.len()).unwrap_or(0),
			idea.creator_name
		);
	}

	if let Some(first) = ideas.first() {
		let fetched = db.get_idea(&first.id);
		println!("get_idea({}): {}", first.id, fetched.is_some());
	}
}
