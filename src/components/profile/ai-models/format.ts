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
