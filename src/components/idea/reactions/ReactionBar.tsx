import { useEffect, useId, useRef, useState } from "react";
import Tooltip from "@ds/Tooltip";
import { cn } from "@helpers/cn";
import { REACTIONS } from "@src/const";

/** One reaction as the bar receives it: the user's own, or someone else's. */
export interface ReactionEntry {
	emoji: string;
	mine: boolean;
	/** sender of someone else's reaction (null: unknown) */
	from?: string | null;
}

/** One chip: every reaction with the same emoji. */
export interface ReactionSummary {
	emoji: string;
	name: string;
	count: number;
	mine: boolean;
	/** who reacted, "You" first */
	people: string[];
}

/**
 * Group reactions by emoji, in the REACTIONS display order (an emoji
 * outside the set, e.g. from an older import, goes last).
 */
export function summarizeReactions(entries: ReactionEntry[]): ReactionSummary[] {
	const byEmoji = new Map<string, { mine: boolean; others: string[] }>();
	for (const entry of entries) {
		const group = byEmoji.get(entry.emoji) ?? { mine: false, others: [] };
		if (entry.mine) {
			group.mine = true;
		} else {
			group.others.push(entry.from ?? "Someone");
		}
		byEmoji.set(entry.emoji, group);
	}
	const order = (emoji: string): number => {
		const index = REACTIONS.findIndex((r) => r.emoji === emoji);
		return index === -1 ? REACTIONS.length : index;
	};
	return [...byEmoji.entries()]
		.map(([emoji, group]) => ({
			emoji,
			name: REACTIONS.find((r) => r.emoji === emoji)?.name ?? "reaction",
			count: (group.mine ? 1 : 0) + group.others.length,
			mine: group.mine,
			people: [...(group.mine ? ["You"] : []), ...group.others]
		}))
		.sort((a, b) => order(a.emoji) - order(b.emoji));
}

interface ReactionBarProps {
	/** what is being reacted to, for the group's accessible name */
	label: string;
	reactions: ReactionEntry[];
	/** toggle the user's own reaction with this emoji */
	onToggle: (emoji: string) => void;
	/** short helper line shown inside the picker */
	note?: string;
	className?: string;
}

/**
 * Reaction chips (emoji + count, the user's own ones pressed) and a
 * "React" button opening a picker of the fixed emoji set. Clicking a chip
 * or a picker emoji only ever toggles the user's own reaction; others'
 * reactions are shown, never changed.
 */
export default function ReactionBar({
	label,
	reactions,
	onToggle,
	note,
	className
}: ReactionBarProps) {
	const [isOpen, setIsOpen] = useState(false);
	const pickerId = useId();
	const wrapperRef = useRef<HTMLDivElement | null>(null);
	const triggerRef = useRef<HTMLButtonElement | null>(null);
	const pickerRef = useRef<HTMLDivElement | null>(null);

	const summary = summarizeReactions(reactions);
	const mine = new Set(reactions.filter((r) => r.mine).map((r) => r.emoji));

	const close = (returnFocus: boolean): void => {
		setIsOpen(false);
		if (returnFocus) triggerRef.current?.focus();
	};

	// Open picker: focus starts on the first emoji; Escape closes and
	// returns focus to "React", arrow keys move between emojis, and a
	// pointer press outside closes it. (No close on blur: WebKit doesn't
	// focus buttons on click, so a blur can precede the emoji's click.)
	useEffect(() => {
		if (!isOpen) return;
		pickerRef.current?.querySelector("button")?.focus();

		const onKeyDown = (event: globalThis.KeyboardEvent): void => {
			if (event.key === "Escape") {
				event.preventDefault();
				setIsOpen(false);
				triggerRef.current?.focus();
				return;
			}
			if (event.key !== "ArrowRight" && event.key !== "ArrowLeft") return;
			const buttons = [...(pickerRef.current?.querySelectorAll("button") ?? [])];
			const current = buttons.indexOf(document.activeElement as HTMLButtonElement);
			if (current === -1) return;
			event.preventDefault();
			const step = event.key === "ArrowRight" ? 1 : -1;
			buttons[(current + step + buttons.length) % buttons.length]?.focus();
		};
		const onPointerDown = (event: PointerEvent): void => {
			if (!wrapperRef.current?.contains(event.target as Node)) {
				setIsOpen(false);
			}
		};
		window.addEventListener("keydown", onKeyDown);
		document.addEventListener("pointerdown", onPointerDown);
		return () => {
			window.removeEventListener("keydown", onKeyDown);
			document.removeEventListener("pointerdown", onPointerDown);
		};
	}, [isOpen]);

	return (
		<div
			role="group"
			aria-label={`Reactions to ${label}`}
			className={cn("flex flex-wrap items-center gap-1 not-prose", className)}
		>
			{summary.map((reaction) => (
				<Tooltip key={reaction.emoji} text={reaction.people.join(", ")}>
					<button
						type="button"
						aria-pressed={reaction.mine}
						aria-label={`${reaction.name} ${reaction.emoji}, ${reaction.count}`}
						onClick={() => onToggle(reaction.emoji)}
						className={cn(
							"inline-flex items-center gap-1 rounded-full border px-2 py-0.5 text-sm leading-tight transition-colors",
							reaction.mine
								? "border-accent-400 bg-accent-50 text-accent-800 hover:bg-accent-100"
								: "border-stone-200 bg-white text-stone-700 hover:bg-stone-100"
						)}
					>
						<span aria-hidden="true">{reaction.emoji}</span>
						<span aria-hidden="true" className="text-xs tabular-nums">
							{reaction.count}
						</span>
					</button>
				</Tooltip>
			))}
			<div ref={wrapperRef} className="relative">
				<button
					ref={triggerRef}
					type="button"
					aria-expanded={isOpen}
					aria-controls={isOpen ? pickerId : undefined}
					onClick={() => setIsOpen((open) => !open)}
					className="inline-flex items-center gap-1 rounded-full border border-dashed border-stone-300 px-2 py-0.5 text-xs leading-tight text-stone-500 hover:border-accent-400 hover:text-accent-700"
				>
					<span aria-hidden="true">+</span>
					React
				</button>
				{isOpen && (
					<div
						ref={pickerRef}
						id={pickerId}
						role="group"
						aria-label="Pick a reaction"
						className="absolute left-0 top-full z-40 mt-1 w-max max-w-[16rem] rounded-lg border border-stone-200 bg-white p-1.5 shadow-lg"
					>
						<div className="flex flex-wrap gap-0.5">
							{REACTIONS.map((reaction) => (
								<button
									key={reaction.emoji}
									type="button"
									aria-pressed={mine.has(reaction.emoji)}
									aria-label={`React with ${reaction.name} ${reaction.emoji}`}
									title={reaction.name}
									onClick={() => {
										onToggle(reaction.emoji);
										close(true);
									}}
									className={cn(
										"rounded-md p-1 text-lg leading-none hover:bg-stone-100",
										mine.has(reaction.emoji) &&
											"bg-accent-50 ring-1 ring-accent-400"
									)}
								>
									<span aria-hidden="true">{reaction.emoji}</span>
								</button>
							))}
						</div>
						{note && (
							<p className="mt-1 px-1 text-[10px] leading-tight text-stone-500">
								{note}
							</p>
						)}
					</div>
				)}
			</div>
		</div>
	);
}
