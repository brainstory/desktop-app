import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import { NotificationsCard } from "./NotificationsCard";
import type { NotificationSetting } from "@helpers/api/settings";
import { mockInvoke } from "@src/test/mock-tauri";
import { SnackbarProvider } from "@ds/Snackbar";

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
			<SnackbarProvider>
				<NotificationsCard notificationsData={[reminder]} saveSettings={async () => true} />
			</SnackbarProvider>
		);
		expect(screen.getByRole("switch", { name: "Daily intention reminder" })).toHaveAttribute(
			"aria-checked",
			"false"
		);
	});

	it("labels the reminder hour with the stored timezone", () => {
		render(
			<SnackbarProvider>
				<NotificationsCard
					notificationsData={[reminder]}
					saveSettings={async () => true}
					timeZone="Asia/Tokyo"
				/>
			</SnackbarProvider>
		);
		// the hour the picker edits is wall-clock time in the user's
		// chosen zone, not the machine's
		expect(screen.getByText("Asia/Tokyo time")).toBeInTheDocument();
	});

	it("edits the hour through the native time input", async () => {
		const save = vi.fn(async () => true);
		render(
			<SnackbarProvider>
				<NotificationsCard notificationsData={[reminder]} saveSettings={save} />
			</SnackbarProvider>
		);
		const input = screen.getByLabelText("Daily intention reminder hour");
		expect(input).toHaveValue("09:00");
		// jsdom does not run the browser's time-widget; fireEvent the change
		fireEvent.change(input, { target: { value: "17:00" } });
		expect(screen.getByRole("button", { name: /save/i })).toBeEnabled();
		await userEvent.click(screen.getByRole("button", { name: /save/i }));
		expect(save).toHaveBeenCalledWith([expect.objectContaining({ value: "17:00:00" })]);
	});

	it("follows a timezone change without remounting", () => {
		const view = render(
			<SnackbarProvider>
				<NotificationsCard
					notificationsData={[reminder]}
					saveSettings={async () => true}
					timeZone="Europe/Berlin"
				/>
			</SnackbarProvider>
		);
		expect(screen.getByText("Europe/Berlin time")).toBeInTheDocument();
		view.rerender(
			<SnackbarProvider>
				<NotificationsCard
					notificationsData={[reminder]}
					saveSettings={async () => true}
					timeZone="Asia/Tokyo"
				/>
			</SnackbarProvider>
		);
		expect(screen.getByText("Asia/Tokyo time")).toBeInTheDocument();
		expect(screen.queryByText("Europe/Berlin time")).not.toBeInTheDocument();
	});

	it("keeps Save available when saving fails, so it can be retried", async () => {
		const user = userEvent.setup();
		const saveSettings = vi.fn(async () => false);
		render(
			<SnackbarProvider>
				<NotificationsCard notificationsData={[reminder]} saveSettings={saveSettings} />
			</SnackbarProvider>
		);
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
		render(
			<SnackbarProvider>
				<NotificationsCard notificationsData={[reminder]} saveSettings={saveSettings} />
			</SnackbarProvider>
		);
		await user.click(screen.getByRole("switch", { name: "Daily intention reminder" }));
		const save = screen.getByRole("button", { name: "Save" });
		await user.click(save);
		await waitFor(() => expect(saveSettings).toHaveBeenCalledOnce());
		expect(save).toBeDisabled();
	});
	it("shows an OS refusal through the snackbar instead of nothing", async () => {
		mockInvoke({
			send_test_notification: () => {
				throw new Error("permission denied");
			}
		});
		render(
			<SnackbarProvider>
				<NotificationsCard notificationsData={[reminder]} saveSettings={async () => true} />
			</SnackbarProvider>
		);
		await userEvent.click(screen.getByRole("button", { name: "Send test notification" }));
		// the real snackbar host shows the refusal (the error styling, not
		// a success message)
		expect(await screen.findByText(/permission denied/)).toBeInTheDocument();
	});
});
