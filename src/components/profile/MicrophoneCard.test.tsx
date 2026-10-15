import { afterEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { invoke } from "@tauri-apps/api/core";

import { mockInvoke } from "@src/test/mock-tauri";
import { MicrophoneCard, levelVerdict } from "./MicrophoneCard";

const devices = [
	{ name: "MacBook Microphone", isDefault: true },
	{ name: "Headset", isDefault: false }
];

describe("levelVerdict", () => {
	it("classifies silence, quiet, healthy and clipping levels", () => {
		expect(levelVerdict(0.001).ok).toBe(false);
		expect(levelVerdict(0.01).ok).toBe(false);
		expect(levelVerdict(0.3).ok).toBe(true);
		expect(levelVerdict(1.0).ok).toBe(false);
	});
});

describe("MicrophoneCard", () => {
	afterEach(() => {
		vi.mocked(invoke).mockClear();
	});

	it("lists devices, saves the choice, and reports the test level", async () => {
		mockInvoke({
			list_input_devices: () => devices,
			get_input_device: () => ({ name: "Headset" }),
			set_input_device: () => null,
			test_input_device: () => ({ peak: 0.42 })
		});
		render(<MicrophoneCard openSnackbar={vi.fn()} />);
		const select = await screen.findByLabelText("Recording device");
		expect(select).toHaveValue("Headset");

		// switching devices persists immediately
		fireEvent.change(select, { target: { value: "" } });
		await waitFor(() => expect(invoke).toHaveBeenCalledWith("set_input_device", { name: "" }));

		// the test recording reports a healthy level
		fireEvent.click(screen.getByRole("button", { name: "Test" }));
		expect(await screen.findByText("Healthy level")).toBeInTheDocument();
	});

	it("warns when the test hears nothing", async () => {
		mockInvoke({
			list_input_devices: () => devices,
			get_input_device: () => ({ name: "" }),
			test_input_device: () => ({ peak: 0.0001 })
		});
		render(<MicrophoneCard openSnackbar={vi.fn()} />);
		await screen.findByLabelText("Recording device");
		fireEvent.click(screen.getByRole("button", { name: "Test" }));
		expect(await screen.findByText(/nothing was picked up/i)).toBeInTheDocument();
	});
});
