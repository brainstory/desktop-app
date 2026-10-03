import { memo } from "react";
import Button from "@ds/Button";
import { useConfirmClick } from "@src/hooks/useTimeout";
import type { ModelStatus } from "@helpers/api/models";
import { formatSize } from "./format";

/** Headroom required on top of the model file before we warn about space */
const DOWNLOAD_SPACE_MARGIN_BYTES = 1_000_000_000;

/**
 * Display percentage for a download: null while the backend could not
 * determine the total size (it reports a negative pct then).
 */
const downloadPercent = (pct: number): number | null =>
	pct < 0 ? null : Math.min(100, Math.floor(pct));

/** Per-model actions; stable identities keep the memoised rows still. */
export interface ModelRowActions {
	onDownload: (modelId: string) => void;
	onCancel: (modelId: string) => void;
	onDelete: (modelId: string) => void;
	onActivate: (modelId: string) => void;
}

interface ModelRowProps extends ModelRowActions {
	model: ModelStatus;
	/** download percentage while downloading (negative = size unknown) */
	progress: number | undefined;
	freeBytes: number | null;
}

/**
 * One catalog model: status, actions and download progress. Memoised so a
 * progress event only re-renders the downloading row.
 */
export const ModelRow = memo(function ModelRow({
	model,
	progress,
	freeBytes,
	onDownload,
	onCancel,
	onDelete,
	onActivate
}: ModelRowProps) {
	const isDownloading = progress !== undefined;
	const pct = progress === undefined ? null : downloadPercent(progress);
	return (
		<div className="flex flex-col gap-2 border border-stone-200 rounded-lg p-4">
			<div className="flex justify-between items-baseline gap-4">
				<div>
					<p className="font-semibold text-stone-900">
						{model.label}
						{model.active && (
							<span className="ml-2 text-xs font-medium text-pink-600 uppercase">
								Active
							</span>
						)}
					</p>
					<p className="text-sm text-stone-500">{model.description}</p>
				</div>
				<div className="flex gap-2 items-center shrink-0">
					{model.downloaded && !model.active && (
						<Button variant="bordered" onClick={() => onActivate(model.id)}>
							Use
						</Button>
					)}
					{model.downloaded && !isDownloading && (
						<DeleteModelButton
							isActive={model.active}
							onConfirm={() => onDelete(model.id)}
						/>
					)}
					{!model.downloaded && !isDownloading && (
						<>
							<Button variant="pink" onClick={() => onDownload(model.id)}>
								Download ({formatSize(model.sizeBytes)})
							</Button>
							{freeBytes !== null && (
								<FreeSpaceNote freeBytes={freeBytes} needBytes={model.sizeBytes} />
							)}
						</>
					)}
					{isDownloading && (
						<Button variant="bordered" onClick={() => onCancel(model.id)}>
							Cancel
						</Button>
					)}
				</div>
			</div>
			{isDownloading && (
				<div className="flex items-center gap-3">
					<div
						role="progressbar"
						aria-label={`${model.label} download progress`}
						aria-valuemin={0}
						aria-valuemax={100}
						aria-valuenow={pct ?? undefined}
						className="w-full bg-stone-200 rounded-full h-2.5 overflow-hidden"
					>
						{pct === null ? (
							// backend couldn't determine the total size
							<div className="bg-pink-500 h-2.5 w-1/3 rounded-full animate-pulse"></div>
						) : (
							<div
								className="bg-pink-500 h-2.5 rounded-full transition-all"
								style={{ width: `${pct}%` }}
							></div>
						)}
					</div>
					<span className="text-xs text-stone-500 tabular-nums shrink-0 w-10 text-right">
						{pct === null ? "…" : `${pct}%`}
					</span>
				</div>
			)}
		</div>
	);
});

/** What every model list needs besides the models themselves. */
export interface ModelListContext {
	downloadProgress: Record<string, number>;
	freeBytes: number | null;
	actions: ModelRowActions;
}

/** A section's catalog models as rows. */
export function ModelList({
	models,
	downloadProgress,
	freeBytes,
	actions
}: ModelListContext & { models: ModelStatus[] }) {
	return models.map((model) => (
		<ModelRow
			key={model.id}
			model={model}
			progress={downloadProgress[model.id]}
			freeBytes={freeBytes}
			{...actions}
		/>
	));
}

/**
 * Two-click delete with a visible countdown (the shared confirm pattern,
 * as on draft cards). The active model gets a stronger warning.
 */
function DeleteModelButton({ isActive, onConfirm }: { isActive: boolean; onConfirm: () => void }) {
	const { isConfirming, secondsLeft, confirm } = useConfirmClick();
	return (
		<Button
			variant="bordered"
			onClick={() => {
				if (confirm()) onConfirm();
			}}
			classes={isConfirming ? "ring-red-400 text-red-600 whitespace-nowrap" : ""}
		>
			{isConfirming ? (
				<span aria-live="polite">
					{`${isActive ? "Really delete the ACTIVE model?" : "Really delete?"} (${secondsLeft ?? 0}s)`}
				</span>
			) : (
				"Delete"
			)}
		</Button>
	);
}

/**
 * Warn-only disk space note next to a Download button: muted when there's
 * enough room, amber when free space is below the model size plus a
 * margin. Never blocks the download - running out mid-transfer fails
 * cleanly through the normal verification path.
 */
function FreeSpaceNote({ freeBytes, needBytes }: { freeBytes: number; needBytes?: number }) {
	if (!needBytes) return null;
	const isLow = freeBytes < needBytes + DOWNLOAD_SPACE_MARGIN_BYTES;
	if (isLow) {
		return (
			<span className="text-xs font-medium text-amber-700 whitespace-nowrap">
				Only {formatSize(freeBytes)} free — needs {formatSize(needBytes)}
			</span>
		);
	}
	return (
		<span className="text-xs text-stone-500 whitespace-nowrap">
			{formatSize(freeBytes)} free
		</span>
	);
}
