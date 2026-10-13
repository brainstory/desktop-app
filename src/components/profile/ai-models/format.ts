/** Display helpers shared by the AI models card's sections. */

export const formatSize = (bytes?: number): string => {
	if (!bytes) return "";
	const gb = bytes / 1_000_000_000;
	if (gb >= 1) return `${gb.toFixed(1)} GB`;
	return `${Math.round(bytes / 1_000_000)} MB`;
};

export const localeLabel = (id: string): string => {
	try {
		return new Intl.DisplayNames([id], { type: "language" }).of(id) ?? id;
	} catch {
		return id;
	}
};

/** Transfer speed, e.g. "12.3 MB/s" / "820 kB/s"; "" when unusable. */
export const formatRate = (bytesPerSec: number): string => {
	if (!(bytesPerSec > 0)) return "";
	const mb = bytesPerSec / 1_000_000;
	if (mb >= 100) return `${Math.round(mb)} MB/s`;
	if (mb >= 1) return `${mb.toFixed(1)} MB/s`;
	return `${Math.max(1, Math.round(bytesPerSec / 1000))} kB/s`;
};

/** Time left, rough: "38s" / "4m 05s" / "1h 20m". */
export const formatEta = (secs: number): string => {
	const total = Math.max(0, Math.round(secs));
	if (total < 60) return `${total}s`;
	const minutes = Math.floor(total / 60);
	if (minutes < 60) {
		return `${minutes}m ${String(total % 60).padStart(2, "0")}s`;
	}
	return `${Math.floor(minutes / 60)}h ${String(minutes % 60).padStart(2, "0")}m`;
};
