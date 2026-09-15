import { hasDoneGettingStarted, hasSeenIndex, markIndexSeen } from "@helpers/storage";

export default function IndexWrapper() {
	// First-run onboarding: send everyone through the guide once.
	if (!hasDoneGettingStarted() && !hasSeenIndex()) {
		markIndexSeen();
		location.href = "/get-started";
	} else {
		location.href = "/dashboard";
	}
}
