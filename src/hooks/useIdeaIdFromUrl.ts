import { useEffect, type Dispatch, type RefObject, type SetStateAction } from "react";
import { getQueryParam } from "@helpers/helpers";
import { QUERY_PARAMS } from "@src/tauri/commands";

/**
 * On first mount only, seed the idea id from the ?id= query param (and
 * mark the mount so later runs of the effect never re-seed). This guards
 * against extra idea saves from edge cases where the ideaId was once in
 * the query string but is not anymore.
 */
export function useIdeaIdFromUrl(
	hasMountedRef: RefObject<boolean>,
	setIdeaId: Dispatch<SetStateAction<string | undefined>>
): void {
	useEffect(() => {
		if (hasMountedRef.current) {
			return;
		}
		setIdeaId(getQueryParam(QUERY_PARAMS.id) ?? undefined);
		hasMountedRef.current = true;
	}, [hasMountedRef, setIdeaId]);
}
