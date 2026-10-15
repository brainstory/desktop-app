import { useEffect, useState } from "react";
import { invokeCommand } from "@src/tauri/invoke";

import Button from "@ds/Button";
import { Card } from "./ProfileCards";

interface InputDevice {
	name: string;
	isDefault: boolean;
}

/** Peak-level verdict for the 2-second test recording. */
export function levelVerdict(peak: number): { label: string; ok: boolean } {
	if (peak < 0.005) return { label: "Silence - nothing was picked up", ok: false };
	if (peak < 0.02) return { label: "Very quiet - check the mic boom or gain", ok: false };
	if (peak > 0.99) return { label: "Clipping - lower the input gain", ok: false };
	return { label: "Healthy level", ok: true };
}

/**
 * Microphone selection: the system default or a named device, with a
 * two-second test recording that reports the peak level so the wrong
 * device is visible before talking into it.
 */
export function MicrophoneCard({
	openSnackbar
}: {
	openSnackbar: (isSuccess: boolean, message: string) => void;
}) {
	const [devices, setDevices] = useState<InputDevice[] | null>(null);
	const [selected, setSelected] = useState<string>("");
	const [isTesting, setIsTesting] = useState(false);
	const [testPeak, setTestPeak] = useState<number | null>(null);

	useEffect(() => {
		let isCurrent = true;
		Promise.all([invokeCommand("listInputDevices"), invokeCommand("getInputDevice")])
			.then(([list, stored]) => {
				if (!isCurrent) return;
				setDevices(list);
				setSelected(stored.name);
			})
			.catch((err) => {
				console.error("could not list microphones", err);
				if (isCurrent) setDevices([]);
			});
		return () => {
			isCurrent = false;
		};
	}, []);

	const save = (name: string): void => {
		setSelected(name);
		invokeCommand("setInputDevice", { name }).catch((err) => {
			console.error("could not save the microphone choice", err);
			openSnackbar(false, "Couldn't save the microphone choice");
		});
	};

	const runTest = (): void => {
		setIsTesting(true);
		setTestPeak(null);
		invokeCommand("testInputDevice")
			.then((result) => setTestPeak(result.peak))
			.catch((err) => {
				console.error("microphone test failed", err);
				openSnackbar(false, "Couldn't record from the microphone");
			})
			.finally(() => setIsTesting(false));
	};

	return (
		<Card columns={3} title="Microphone">
			<div className="flex flex-col gap-3">
				<label htmlFor="mic-select" className="text-sm font-medium text-stone-900">
					Recording device
				</label>
				<select
					id="mic-select"
					value={selected}
					onChange={(e) => save(e.target.value)}
					className="border border-stone-300 text-stone-900 text-sm rounded-lg focus:ring-accent-500 focus:border-accent-500 block w-full p-2 bg-white"
				>
					<option value="">System default</option>
					{(devices ?? []).map((device) => (
						<option key={device.name} value={device.name}>
							{device.name}
							{device.isDefault ? " (default)" : ""}
						</option>
					))}
				</select>
				<div className="flex items-center gap-3">
					<Button variant="bordered" onClick={runTest} disabled={isTesting}>
						{isTesting ? "Recording 2s..." : "Test"}
					</Button>
					{testPeak !== null && (
						<span
							role="status"
							className={
								levelVerdict(testPeak).ok
									? "text-sm text-green-700"
									: "text-sm text-amber-700"
							}
						>
							{levelVerdict(testPeak).label}
						</span>
					)}
				</div>
				<p className="text-xs text-stone-500">
					The test records two seconds and reports the loudest moment. The choice applies
					to the next recording.
				</p>
			</div>
		</Card>
	);
}

export default MicrophoneCard;
