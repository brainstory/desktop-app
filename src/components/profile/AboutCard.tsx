import { useEffect, useState } from "react";
import { getVersion } from "@tauri-apps/api/app";
import { invokeCommand } from "@src/tauri/invoke";

import Button from "@ds/Button";
import { Card } from "./ProfileCards";

interface AboutCardProps {
	/** snackbar host (copied diagnostics / failures) */
	openSnackbar: (isSuccess: boolean, message: string) => void;
}

/** Version, support shortcuts and a diagnostics bundle for support
 * requests. The folder buttons run as Rust commands (the opener plugin
 * stays off the webview capability list). */
export function AboutCard({ openSnackbar }: AboutCardProps) {
	const [version, setVersion] = useState<string | null>(null);

	useEffect(() => {
		let isCurrent = true;
		getVersion()
			.then((v) => {
				if (isCurrent) setVersion(v);
			})
			.catch((err) => {
				console.error("could not read the app version", err);
			});
		return () => {
			isCurrent = false;
		};
	}, []);

	const handleOpenDataFolder = () => {
		invokeCommand("openDataFolder").catch((err) =>
			openSnackbar(false, `Couldn't open the data folder: ${String(err)}`)
		);
	};

	const handleOpenLogFolder = () => {
		invokeCommand("openLogFolder").catch((err) =>
			openSnackbar(false, `Couldn't open the log folder: ${String(err)}`)
		);
	};

	const handleCopyDiagnostics = () => {
		invokeCommand("copyDiagnostics")
			.then((report) => navigator.clipboard.writeText(report))
			.then(() => openSnackbar(true, "Diagnostics copied"))
			.catch((err) => {
				console.error("could not collect diagnostics", err);
				openSnackbar(false, "Couldn't collect diagnostics");
			});
	};

	return (
		<Card columns={3} title="Diagnostics">
			<div className="flex flex-col gap-3">
				<p className="text-sm text-stone-700">
					Brainstory {version ?? "…"} — everything stays on this device.
				</p>
				<div className="flex flex-wrap gap-2">
					<Button variant="bordered" onClick={handleOpenLogFolder}>
						Open log folder
					</Button>
					<Button variant="bordered" onClick={handleOpenDataFolder}>
						Open data folder
					</Button>
					<Button variant="bordered" onClick={handleCopyDiagnostics}>
						Copy diagnostics
					</Button>
				</div>
			</div>
		</Card>
	);
}
