/**
 * Format-on-edit for pi — mirrors `.claude/hooks/format-on-edit.sh`.
 *
 * After a `write` or `edit` tool modifies a Rust source file, runs
 * `rustfmt --edition 2024` so the working tree stays formatted. Best-effort:
 * a formatting failure is surfaced as a notification, never as a tool error,
 * so the agent is never blocked by a formatting issue.
 *
 * Loaded after project trust from `.pi/extensions/`. Use /trust to save the
 * decision for this worktree so it loads without prompting.
 */

import { spawn } from "node:child_process";
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";

const RUSTFMT_TIMEOUT_MS = 10_000;

export default function (pi: ExtensionAPI) {
	pi.on("tool_result", async (event, ctx) => {
		if (event.toolName !== "write" && event.toolName !== "edit") return undefined;

		// Don't bother formatting if the edit itself failed.
		if (event.isError) return undefined;

		const path = String(event.input.path ?? "");
		if (!path.endsWith(".rs")) return undefined;

		try {
			await new Promise<void>((resolve) => {
				const child = spawn("rustfmt", ["--edition", "2024", path], {
					stdio: ["ignore", "ignore", "pipe"],
					signal: ctx.signal,
				});

				// Capture stderr so the warning can say why rustfmt failed instead
				// of just "exited 1".
				let stderr = "";
				child.stderr?.on("data", (chunk: Buffer | string) => {
					stderr += chunk.toString();
				});

				let notified = false;
				const warn = (msg: string) => {
					if (!notified && !ctx.signal?.aborted) {
						notified = true;
						ctx.ui.notify?.(msg, "warning");
					}
				};

				let settled = false;
				let timer: ReturnType<typeof setTimeout> | undefined;
				const finish = () => {
					if (!settled) {
						settled = true;
						if (timer) clearTimeout(timer);
						resolve();
					}
				};

				// A wedged rustfmt would otherwise stall the agent turn. Kill it and
				// surface a warning; 'close' fires after the kill and resolves.
				timer = setTimeout(() => {
					child.kill("SIGKILL");
					warn(`rustfmt timed out after ${RUSTFMT_TIMEOUT_MS}ms for ${path}`);
				}, RUSTFMT_TIMEOUT_MS);

				child.on("error", () => {
					if (!ctx.signal?.aborted) warn(`rustfmt failed to run for ${path}`);
					finish();
				});
				child.on("close", (code) => {
					if (code !== 0 && !ctx.signal?.aborted) {
						const detail = stderr.trim().split("\n").pop() ?? "";
						warn(`rustfmt exited ${code ?? "<killed>"} for ${path}${detail ? `: ${detail}` : ""}`);
					}
					finish();
				});
			});
		} catch {
			// Never block the agent on a formatting failure.
		}

		return undefined;
	});
}
