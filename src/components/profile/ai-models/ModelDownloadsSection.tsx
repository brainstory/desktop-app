import { useId, useState } from "react";
import Button from "@ds/Button";
import type { AiSettingsResponse } from "@helpers/api/models";
import { SecretRow } from "./SecretRow";

interface ModelDownloadsSectionProps {
	settings: AiSettingsResponse;
	saveSecret: (key: string, value: string) => Promise<boolean> | void;
	saveEndpoint: (value: string) => void;
}

/** Where local models come from: applies to language and whisper models alike. */
export function ModelDownloadsSection({
	settings,
	saveSecret,
	saveEndpoint
}: ModelDownloadsSectionProps) {
	const headingId = useId();
	return (
		<section aria-labelledby={headingId} className="flex flex-col gap-3">
			<h3 id={headingId} className="font-semibold">
				Model downloads
			</h3>
			<p className="text-sm text-stone-500 -mt-2">
				Applies to every model downloaded on this page, language and speech alike.
			</p>
			<div className="border border-stone-200 rounded-lg p-4 text-sm">
				<SecretRow
					inputId="hf-token-input"
					label="HuggingFace access token (optional)"
					labelClassName="font-semibold mb-1 block"
					description={
						<>
							Authenticated downloads are faster and never hit HuggingFace&rsquo;s
							anonymous rate limits. Create a free read token at
							huggingface.co/settings/tokens.
						</>
					}
					placeholder="hf_..."
					stored={settings.hfTokenSet}
					hint={settings.hfTokenHint}
					saveLabel="Save Token"
					onSave={(value) => saveSecret("hfToken", value)}
				/>
				<label htmlFor="hf-endpoint-input" className="font-semibold mb-1 mt-4 block">
					HuggingFace download endpoint (mirror, optional)
				</label>
				<p className="text-stone-500 mb-2">
					Leave empty to download from huggingface.co directly, or point at a mirror (e.g.
					https://hf-mirror.com). Also picks up the HF_ENDPOINT environment variable when
					launched from a terminal. Your access token is sent to the mirror too.
				</p>
				<EndpointField
					inputId="hf-endpoint-input"
					initial={settings.hfEndpoint}
					onSave={saveEndpoint}
				/>
			</div>
		</section>
	);
}

/** Plain-text field with its own Save button for a single non-secret
 * setting (the download endpoint/mirror). Sends only its own value. */
function EndpointField({
	inputId,
	initial,
	onSave
}: {
	inputId: string;
	initial?: string;
	onSave: (value: string) => void;
}) {
	const [value, setValue] = useState(initial ?? "");
	return (
		<div className="flex flex-wrap gap-2">
			<input
				id={inputId}
				type="url"
				value={value}
				onChange={(e) => setValue(e.target.value)}
				placeholder="https://huggingface.co"
				className="border border-stone-300 text-stone-900 text-sm rounded-lg focus:ring-accent-500 focus:border-accent-500 flex-1 min-w-0 p-2"
			/>
			<Button
				variant="pink"
				disabled={value === (initial ?? "")}
				onClick={() => onSave(value)}
			>
				Save
			</Button>
		</div>
	);
}
