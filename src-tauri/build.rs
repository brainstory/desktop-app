fn main() {
	guard_macos_sdk();
	tauri_build::build()
}

/// The `speech` crate compiles its macOS 26 SpeechAnalyzer bridge against
/// whatever SDK the active Xcode provides; an older SDK silently stubs the
/// analyzer APIs out (transcription then errors at runtime and the app falls
/// back to whisper). Warn locally, and fail hard in CI where a stubbed build
/// must never ship.
fn guard_macos_sdk() {
	if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
		return;
	}
	let output = std::process::Command::new("xcrun")
		.args(["--sdk", "macosx", "--show-sdk-version"])
		.output();
	let major = output.ok().filter(|o| o.status.success()).and_then(|o| {
		String::from_utf8_lossy(&o.stdout)
			.trim()
			.split('.')
			.next()
			.and_then(|m| m.parse::<u32>().ok())
	});
	match major {
		Some(m) if m >= 26 => {}
		_ => {
			let found = major
				.map(|m| m.to_string())
				.unwrap_or_else(|| "unknown".into());
			if std::env::var("REQUIRE_MACOS26_SDK").is_ok() {
				panic!(
					"the macOS SDK is {found} but >= 26 is required: the Apple Speech \
					 engine would be compiled as a stub and never work"
				);
			}
			println!(
				"cargo:warning=macOS SDK {found} < 26: Apple Speech (SpeechAnalyzer) \
				 support will be compiled as a stub; transcription falls back to whisper"
			);
		}
	}
}
