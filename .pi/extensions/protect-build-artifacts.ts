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

import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";

const PROTECTED = [
	"src/generated", // protobuf codegen output (run `make -C src generated`)
	"/target/", // cargo build output
	"Cargo.lock", // workspace lockfiles
	"codegen/", // tonic_build script (regenerate, don't hand-edit)
];

export default function (pi: ExtensionAPI) {
	pi.on("tool_call", async (event, ctx) => {
		if (event.toolName !== "write" && event.toolName !== "edit") return undefined;

		const path = String(event.input.path ?? "");
		if (!path) return undefined;

		if (PROTECTED.some((p) => path.includes(p))) {
			ctx.ui.notify?.(`Blocked write to build artifact: ${path}`, "warning");
			return { block: true, reason: `"${path}" is a generated/build artifact; regenerate via \`make -C src generated\` instead of editing` };
		}

		return undefined;
	});
}
