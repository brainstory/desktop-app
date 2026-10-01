fn main() {
	println!("cargo:rerun-if-env-changed=REQUIRE_MACOS26_SDK");
	guard_macos_sdk();
	link_clang_runtime();
	tauri_build::build()
}

/// The vendored ggml Metal code (in both llama.cpp and whisper.cpp) uses
/// Objective-C `@available` checks, which compile to calls into clang's
/// runtime (`___isPlatformVersionAtLeast` in libclang_rt.osx.a). rustc
/// links with -nodefaultlibs, so nothing pulls that library in by itself.
/// llama-cpp-sys-2 up to 0.1.156 linked it from its build script and
/// whisper-rs-sys silently relied on that; 0.1.157 dropped it and release
/// links fail with "Undefined symbols: ___isPlatformVersionAtLeast". This
/// binary is what combines the two engines, so it links the runtime.
fn link_clang_runtime() {
	if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
		return;
	}
	// `clang --print-search-dirs` prints "libraries: =<resource dir>";
	// the runtime lives in <resource dir>/lib/darwin
	let output = std::process::Command::new("clang")
		.arg("--print-search-dirs")
		.output();
	let dir = output.ok().filter(|o| o.status.success()).and_then(|o| {
		String::from_utf8_lossy(&o.stdout)
			.lines()
			.find_map(|line| line.strip_prefix("libraries: =").map(str::to_owned))
	});
	match dir {
		Some(dir) => {
			println!("cargo:rustc-link-search=native={dir}/lib/darwin");
			println!("cargo:rustc-link-lib=clang_rt.osx");
		}
		None => println!(
			"cargo:warning=could not locate clang's runtime library; linking may fail \
			 with an undefined ___isPlatformVersionAtLeast"
		),
	}
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
