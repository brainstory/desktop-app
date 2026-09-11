import { getCookie, setCookie } from "@src/helpers/cookie";

export default function IndexWrapper() {
	// First-run onboarding: send everyone through the guide once.
	if (getCookie("has_done_getting_started") === undefined && getCookie("has_seen_index") === undefined) {
		setCookie("has_seen_index", true, 1);
		location.href = "/get-started";
	} else {
		location.href = "/dashboard";
	}
}
