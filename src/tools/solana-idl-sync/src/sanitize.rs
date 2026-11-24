//! Redaction of the RPC URL (it carries the API key) and fence
//! neutralization for text derived from transactions.

/// Replace every backtick in `text` with a single quote, so text derived
/// from untrusted sources (transaction instruction titles, condensed rows,
/// parse errors, provenance-adjacent IDL names) can never open or close a
/// fenced code block in the report.
pub fn neutralize_backticks(text: &str) -> String {
    text.replace('`', "'")
}

/// A fenced code block around untrusted `content` (backticks neutralized),
/// indented for the surrounding markdown.
pub fn fenced(content: &str) -> String {
    format!("```\n{}\n```", neutralize_backticks(content))
}

/// Replace every occurrence of `secret` in `text` with `<redacted>`. An
/// empty secret is a no-op (nothing to redact; `str::replace("")` would
/// otherwise splice a marker between every character).
pub fn redact(text: &str, secret: &str) -> String {
    if secret.is_empty() {
        return text.to_string();
    }
    text.replace(secret, "<redacted>")
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn backtick_runs_cannot_close_a_fence() {
        let hostile = "before ```\n<script>\nafter ````x";
        let neutral = neutralize_backticks(hostile);
        assert!(!neutral.contains('`'), "{neutral}");
        // Distinct inputs stay distinct; nothing is dropped.
        assert_eq!(neutral, "before '''\n<script>\nafter ''''x");
        let block = fenced(hostile);
        assert_eq!(block, "```\nbefore '''\n<script>\nafter ''''x\n```");
        // Only the delimiters of the block itself are backticks, and a
        // hostile line can never end the block early.
        assert_eq!(block.matches("```").count(), 2);
    }

    #[test]
    fn redact_removes_every_occurrence() {
        let url = "https://mainnet.helius-rpc.com/?api-key=SECRET";
        let err = format!("RPC request to {url} failed: connection refused");
        assert_eq!(
            redact(&err, url),
            "RPC request to <redacted> failed: connection refused"
        );
        // Only the full secret is a secret; fragments stay visible.
        assert_eq!(
            redact("api-key=SECRET and SECRET", "api-key=SECRET"),
            "<redacted> and SECRET"
        );
        assert_eq!(redact("nothing", "not-present"), "nothing");
    }

    #[test]
    fn redact_with_empty_secret_is_a_no_op() {
        assert_eq!(redact("a b c", ""), "a b c");
    }
}
