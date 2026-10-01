import { invoke, type InvokeArgs } from "@tauri-apps/api/core";
import { COMMANDS, type CommandMap } from "./commands";

export type CommandKey = keyof CommandMap;
export type CommandArgs<K extends CommandKey> = CommandMap[K]["args"];
export type CommandResult<K extends CommandKey> = CommandMap[K]["result"];

/** No args parameter for commands that take none; a required one otherwise. */
type ArgsParam<K extends CommandKey> =
	CommandArgs<K> extends undefined ? [] : [args: CommandArgs<K>];

/**
 * Typed `invoke`: `name` is a COMMANDS key, and the args and result
 * types come from CommandMap.
 *
 *   const idea = await invokeCommand("getIdea", { ideaId });
 */
export function invokeCommand<K extends CommandKey>(
	name: K,
	...args: ArgsParam<K>
): Promise<CommandResult<K>> {
	const command = COMMANDS[name];
	// no-arg commands are invoked without an args parameter at all
	return args.length === 0
		? invoke<CommandResult<K>>(command)
		: invoke<CommandResult<K>>(command, args[0] as InvokeArgs);
}
