import { describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import SecretField from "./SecretField";

describe("SecretField", () => {
	it("clears the typed value only after a confirmed save", async () => {
		const user = userEvent.setup();
		const onSave = vi.fn(async () => true);
		const { container } = render(
			<SecretField inputId="k" placeholder="sk-..." onSave={onSave} />
		);
		const input = container.querySelector("input")!;
		await user.type(input, "sk-secret");
		await user.click(screen.getByRole("button", { name: "Save" }));
		expect(onSave).toHaveBeenCalledWith("sk-secret");
		expect(input).toHaveValue("");
	});

	it("keeps a rejected value for a retry and says so", async () => {
		const user = userEvent.setup();
		const onSave = vi.fn(async () => false);
		const { container } = render(<SecretField inputId="k" onSave={onSave} />);
		const input = container.querySelector("input")!;
		await user.type(input, "sk-secret");
		await user.click(screen.getByRole("button", { name: "Save" }));
		expect(await screen.findByRole("alert")).toHaveTextContent(/Did not save/);
		expect(input).toHaveValue("sk-secret");
	});

	it("shows the stored state with its hint and clears through Remove", async () => {
		const user = userEvent.setup();
		const onSave = vi.fn(async () => true);
		render(<SecretField inputId="k" stored hint="••••1234" onSave={onSave} />);
		expect(screen.getByText(/Saved \(••••1234\)/)).toBeInTheDocument();
		// the placeholder tells the user an empty field keeps the value
		expect(screen.getByPlaceholderText(/Leave empty to keep/)).toBeInTheDocument();
		await user.click(screen.getByRole("button", { name: "Remove" }));
		expect(onSave).toHaveBeenCalledWith("");
	});

	it("ignores a second save while the first is pending", async () => {
		const user = userEvent.setup();
		let resolve!: (ok: boolean) => void;
		const onSave = vi.fn(() => new Promise<boolean>((res) => (resolve = res)));
		const { container } = render(<SecretField inputId="k" onSave={onSave} />);
		const input = container.querySelector("input")!;
		await user.type(input, "sk-secret");
		const save = screen.getByRole("button", { name: "Save" });
		await user.click(save);
		expect(save).toBeDisabled();
		await user.click(save);
		expect(onSave).toHaveBeenCalledTimes(1);
		resolve(true);
		await waitFor(() => expect(input).toHaveValue(""));
	});
});
