import { describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

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

	it("labels the reminder hour with the stored timezone", () => {
		render(
			<NotificationsCard
				notificationsData={[reminder]}
				saveSettings={async () => true}
				timeZone="Asia/Tokyo"
			/>
		);
		// the hour the picker edits is wall-clock time in the user's
		// chosen zone, not the machine's
		expect(screen.getByText("Asia/Tokyo time")).toBeInTheDocument();
	});

	it("follows a timezone change without remounting", () => {
		const view = render(
			<NotificationsCard
				notificationsData={[reminder]}
				saveSettings={async () => true}
				timeZone="Europe/Berlin"
			/>
		);
		expect(screen.getByText("Europe/Berlin time")).toBeInTheDocument();
		view.rerender(
			<NotificationsCard
				notificationsData={[reminder]}
				saveSettings={async () => true}
				timeZone="Asia/Tokyo"
			/>
		);
		expect(screen.getByText("Asia/Tokyo time")).toBeInTheDocument();
		expect(screen.queryByText("Europe/Berlin time")).not.toBeInTheDocument();
	});

	it("keeps Save available when saving fails, so it can be retried", async () => {
		const user = userEvent.setup();
		const saveSettings = vi.fn(async () => false);
		render(<NotificationsCard notificationsData={[reminder]} saveSettings={saveSettings} />);
		const save = screen.getByRole("button", { name: "Save" });
		expect(save).toBeDisabled();
		await user.click(screen.getByRole("switch", { name: "Daily intention reminder" }));
		await user.click(save);
		expect(saveSettings).toHaveBeenCalledWith([{ ...reminder, enabled: true }]);
		await waitFor(() => expect(save).toBeEnabled());
		await user.click(save);
		expect(saveSettings).toHaveBeenCalledTimes(2);
	});

	it("disables Save again after a successful save", async () => {
		const user = userEvent.setup();
		const saveSettings = vi.fn(async () => true);
		render(<NotificationsCard notificationsData={[reminder]} saveSettings={saveSettings} />);
		await user.click(screen.getByRole("switch", { name: "Daily intention reminder" }));
		const save = screen.getByRole("button", { name: "Save" });
		await user.click(save);
		await waitFor(() => expect(saveSettings).toHaveBeenCalledOnce());
		expect(save).toBeDisabled();
	});
});
