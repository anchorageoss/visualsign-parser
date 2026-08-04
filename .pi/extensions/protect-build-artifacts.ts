/**
 * Project-local build-artifact guard for visualsign-parser.
 *
 * Blocks the agent from writing/editing generated or build-output paths that
 * must not be hand-edited (see CLAUDE.md). This complements the global
 * protected-paths extension (which guards secrets, keys, .git, etc.).
 *
 * Note: this only intercepts the `write` and `edit` tools. A `bash` call (e.g.
 * `sed -i`) can still reach these paths, so treat this as a guardrail, not an
 * airtight sandbox.
 *
 * Matching is done against the path relative to the project root (ctx.cwd),
 * normalized to forward slashes, so the check is independent of where the
 * checkout lives (e.g. a checkout under `~/target/visualsign-parser` no longer
 * trips the `/target/` rule) and of the platform's path separators.
 *
 * Loaded after project trust from .pi/extensions/. Use /trust to save the
 * decision for this worktree so it loads without prompting.
 */

import { relative, resolve, sep } from "node:path";
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";

// `codegen/` is intentionally NOT protected: it is the hand-written generator
// crate (src/codegen/src/main.rs), not generated output. Only its output crate
// `src/generated` is protected.
const PROTECTED: ReadonlyArray<{
	reason: string;
	match: (rel: string) => boolean;
}> = [
	{
		reason: "protobuf codegen output; regenerate via `make -C src generated`",
		match: (rel) => rel === "src/generated" || rel.startsWith("src/generated/"),
	},
	{
		// Any real `target` path segment (workspace root or a per-crate dir),
		// matched as a segment rather than a substring so `my-target/` is left
		// alone.
		reason: "cargo build output",
		match: (rel) => rel === "target" || rel.startsWith("target/") || rel.includes("/target/"),
	},
	{
		// Workspace root lockfile only. A nested lockfile (e.g. `fuzz/Cargo.lock`)
		// is a separate per-crate file, so it is left alone and the reason stays
		// accurate.
		reason: "workspace lockfile",
		match: (rel) => rel === "Cargo.lock",
	},
];

// Repo-relative, forward-slash path. resolve() handles both absolute and
// relative tool inputs; relative() re-roots them at the project dir so a path
// outside the repo can't match (it yields a `..` prefix).
function toRel(cwd: string, raw: string): string {
	const rel = relative(cwd, resolve(cwd, raw));
	return sep === "/" ? rel : rel.split(sep).join("/");
}

export default function (pi: ExtensionAPI) {
	pi.on("tool_call", async (event, ctx) => {
		if (event.toolName !== "write" && event.toolName !== "edit") return undefined;

		const path = String(event.input.path ?? "");
		if (!path) return undefined;

		const match = PROTECTED.find((p) => p.match(toRel(ctx.cwd, path)));
		if (match) {
			ctx.ui.notify?.(`Blocked write to build artifact: ${path}`, "warning");
			return { block: true, reason: `"${path}" is ${match.reason}` };
		}

		return undefined;
	});
}
