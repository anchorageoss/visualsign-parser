/**
 * Project-local build-artifact guard for visualsign-parser.
 *
 * Blocks the agent from writing/editing generated or build-output paths that
 * must not be hand-edited (see CLAUDE.md). This complements the global
 * protected-paths extension (which guards secrets, keys, .git, etc.).
 *
 * Loaded after project trust from .pi/extensions/. Use /trust to save the
 * decision for this worktree so it loads without prompting.
 */

import { resolve } from "node:path";
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";

// `codegen/` is intentionally NOT protected: it is the hand-written generator
// crate (src/codegen/src/main.rs), not generated output. Only its output
// crate `src/generated` is protected. Paths are resolved to absolute before
// matching so both relative ("target/...") and absolute tool inputs are caught.
const PROTECTED: ReadonlyArray<{ pattern: string; reason: string }> = [
	{ pattern: "src/generated", reason: "protobuf codegen output; regenerate via `make -C src generated`" },
	{ pattern: "/target/", reason: "cargo build output" },
	{ pattern: "Cargo.lock", reason: "workspace lockfile" },
];

export default function (pi: ExtensionAPI) {
	pi.on("tool_call", async (event, ctx) => {
		if (event.toolName !== "write" && event.toolName !== "edit") return undefined;

		const path = String(event.input.path ?? "");
		if (!path) return undefined;

		const resolved = resolve(path);
		const match = PROTECTED.find((p) => resolved.includes(p.pattern));
		if (match) {
			ctx.ui.notify?.(`Blocked write to build artifact: ${path}`, "warning");
			return { block: true, reason: `"${path}" is ${match.reason}` };
		}

		return undefined;
	});
}
