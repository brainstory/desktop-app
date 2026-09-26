//! Runtime capability probes for Apple's built-in speech recognition
//! (SpeechAnalyzer, macOS 26+). Every `speech`-crate call is gated behind
//! `os_at_least_26()` so the (weak-floored) Swift bridge is never touched on
//! older systems; the OS check itself only uses libc sysctl.

#[cfg(target_os = "macos")]
pub use macos::*;

#[cfg(target_os = "macos")]
mod macos {
	use std::sync::OnceLock;

	/// Product version string from kern.osproductversion (e.g. "26.6.1"),
	/// empty when the sysctl is unavailable.
	fn os_product_version() -> String {
		let mut buf = [0u8; 32];
		let mut len = buf.len();
		let ok = unsafe {
			libc::sysctlbyname(
				c"kern.osproductversion".as_ptr() as *const libc::c_char,
				buf.as_mut_ptr() as *mut libc::c_void,
				&mut len,
				std::ptr::null_mut(),
				0,
			)
		};
		if ok != 0 || len == 0 {
			return String::new();
		}
		String::from_utf8_lossy(&buf[..len.saturating_sub(1)])
			.trim()
			.to_string()
	}

	pub fn os_at_least_26() -> bool {
		static CACHE: OnceLock<bool> = OnceLock::new();
		*CACHE.get_or_init(|| {
			os_product_version()
				.split('.')
				.next()
				.and_then(|major| major.parse::<u32>().ok())
				.is_some_and(|major| major >= 26)
		})
	}

	/// True when the macOS 26 SpeechTranscriber can be used at runtime.
	/// False on older macOS, non-Apple-Silicon, and builds made with a
	/// pre-26 SDK (the bridge stubs report unavailable).
	pub fn speech_available() -> bool {
		static CACHE: OnceLock<bool> = OnceLock::new();
		*CACHE.get_or_init(|| {
			if !os_at_least_26() {
				return false;
			}
			// Safe even if built with an older SDK: the stub returns false.
			speech::analyzer::SpeechTranscriber::is_available()
		})
	}
}

#[cfg(not(target_os = "macos"))]
pub use other::*;

#[cfg(not(target_os = "macos"))]
mod other {
	pub fn os_at_least_26() -> bool {
		false
	}

	pub fn speech_available() -> bool {
		false
	}
}
