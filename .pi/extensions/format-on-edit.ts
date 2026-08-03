/**
 * Format-on-edit for pi — mirrors `.claude/hooks/format-on-edit.sh`.
 *
 * After a `write` or `edit` tool modifies a Rust source file, runs
 * `rustfmt --edition 2024` so the working tree stays formatted. Best-effort:
 * a formatting failure is surfaced as a notification, never as a tool error,
 * so the agent is never blocked by a formatting issue.
 *
 * Loaded after project trust from `.pi/extensions/`. Use `/trust` to save the
 * decision for this worktree so it loads without prompting.
 */

import { spawn } from "node:child_process";
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";

export default function (pi: ExtensionAPI) {
	pi.on("tool_result", async (event, ctx) => {
		if (event.toolName !== "write" && event.toolName !== "edit") return undefined;

		// Don't bother formatting if the edit itself failed.
		if (event.isError) return undefined;

		const path = String(event.input.path ?? "");
		if (!path.endsWith(".rs")) return undefined;

		await new Promise<void>((resolve) => {
			const child = spawn("rustfmt", ["--edition", "2024", path], {
				stdio: "ignore",
				signal: ctx.signal,
			});
			let notified = false;
			const warn = (msg: string) => {
				if (!notified) {
					notified = true;
					ctx.ui.notify?.(msg, "warning");
				}
			};
			child.on("error", () => {
				if (!ctx.signal?.aborted) warn(`rustfmt failed to run for ${path}`);
				resolve();
			});
			child.on("close", (code) => {
				if (code !== 0 && !ctx.signal?.aborted) warn(`rustfmt exited ${code ?? "<killed>"} for ${path}`);
				resolve();
			});
		});

		return undefined;
	});
}
