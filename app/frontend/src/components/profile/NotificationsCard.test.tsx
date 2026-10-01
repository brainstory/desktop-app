import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";

import { NotificationsCard } from "./NotificationsCard";
import type { NotificationSetting } from "@helpers/api/settings";

const reminder: NotificationSetting = {
	title: "Daily intention reminder",
	description: "A daily reminder.",
	value: "09:00:00",
	valueType: "time",
	enabled: false
};

describe("NotificationsCard", () => {
	it("names each switch after its notification", () => {
		render(
			<NotificationsCard notificationsData={[reminder]} saveSettings={async () => true} />
		);
		expect(screen.getByRole("switch", { name: "Daily intention reminder" })).toHaveAttribute(
			"aria-checked",
			"false"
		);
	});
});
