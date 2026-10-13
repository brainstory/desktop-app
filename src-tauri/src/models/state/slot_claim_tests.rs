use super::EngineSlotClaim;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[test]
fn a_claim_waits_for_the_running_load_and_blocks_new_ones() {
	let loading = Arc::new(AtomicBool::new(true)); // a load is running
	let running = loading.clone();
	let finisher = std::thread::spawn(move || {
		std::thread::sleep(Duration::from_millis(200));
		running.store(false, Ordering::SeqCst); // the load finishes
	});
	let started = Instant::now();
	let Some(claim) = EngineSlotClaim::acquire(&loading, Duration::from_secs(5)) else {
		panic!("the claim must succeed once the running load finishes");
	};
	assert!(
		started.elapsed() >= Duration::from_millis(150),
		"the claim must wait for the running load"
	);
	// while held, a new load is refused (load_llm/load_stt swap the flag)
	assert!(loading.swap(true, Ordering::SeqCst), "slot is held");
	drop(claim);
	assert!(!loading.load(Ordering::SeqCst), "released on drop");
	finisher.join().unwrap();
}

#[test]
fn a_wedged_slot_times_out_to_none_and_stays_owned_by_its_holder() {
	let loading = AtomicBool::new(true);
	let claim = EngineSlotClaim::acquire(&loading, Duration::from_millis(100));
	assert!(claim.is_none(), "a wedged slot must not authorize mutation");
	assert!(
		loading.load(Ordering::SeqCst),
		"an unclaimed slot stays owned by whoever holds it"
	);
}
