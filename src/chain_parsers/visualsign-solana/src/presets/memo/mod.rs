//! SPL Memo preset implementation for Solana.
//!
//! The Memo programs (v2 `MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr` and the
//! still-live v1 `Memo1UhkJRfHyvLMcVucJwxXeuD728EqVDDwQDxFMNo`) are native,
//! non-Anchor programs: their instruction data carries no 8-byte discriminator
//! and no borsh-encoded arguments. The entire data buffer *is* the memo. The
//! memo is chosen by whoever built the transaction, so it is shown escaped
//! (`escape_display_bytes`), never raw: a newline or bidi control could
//! otherwise forge extra rows in the approval display. This visualizer
//! therefore decodes the data directly as text rather than going through the
//! IDL/discriminator path used by Anchor-program presets (e.g. dflow_aggregator).
//! It mirrors the native-program pattern of `compute_budget` and
//! `associated_token_account`.

mod config;

use crate::core::{
    InstructionVisualizer, ProgramRef, SolanaIntegrationConfig, VisualizerContext, VisualizerKind,
};
use crate::idl::builtin_programs::canonical_name;
use config::MemoConfig;
use visualsign::encodings::escape_display_bytes;
use visualsign::errors::VisualSignError;
use visualsign::field_builders::{create_raw_data_field, create_text_field};
use visualsign::{
    AnnotatedPayloadField, SignablePayloadField, SignablePayloadFieldCommon,
    SignablePayloadFieldListLayout, SignablePayloadFieldPreviewLayout, SignablePayloadFieldTextV2,
};

pub(crate) const MEMO_PROGRAM_ID: &str = "MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr";
pub(crate) const MEMO_V1_PROGRAM_ID: &str = "Memo1UhkJRfHyvLMcVucJwxXeuD728EqVDDwQDxFMNo";

/// Display name for a memo program, from `idl::builtin_programs::canonical_name`
/// so it cannot drift from the canonical table.
fn program_display_name(program_id: &str) -> &'static str {
    canonical_name(program_id).unwrap_or("Memo Program")
}

static MEMO_CONFIG: MemoConfig = MemoConfig;

pub struct MemoVisualizer;

impl InstructionVisualizer for MemoVisualizer {
    fn visualize_tx_commands(
        &self,
        context: &VisualizerContext,
    ) -> Result<AnnotatedPayloadField, VisualSignError> {
        let program_id = match context.program_id() {
            ProgramRef::Resolved(pk) => pk.to_string(),
            ProgramRef::Unresolved { raw_index } => format!("unresolved({raw_index})"),
        };
        render_memo(&program_id, context.data(), context.instruction_index())
    }

    fn get_config(&self) -> Option<&dyn SolanaIntegrationConfig> {
        Some(&MEMO_CONFIG)
    }

    fn kind(&self) -> VisualizerKind {
        VisualizerKind::Payments("Memo")
    }
}

/// Render the memo bytes for display, escaped.
///
/// Every byte that isn't printable ASCII (newlines, control and bidi
/// characters, non-ASCII UTF-8, invalid UTF-8) is percent-encoded, so the
/// text can't add rows to the display, always passes the payload charset
/// check, and distinct signed bytes always render differently.
/// `unescape_display_bytes` recovers the exact bytes.
fn memo_display_text(data: &[u8]) -> String {
    escape_display_bytes(data)
}

/// The memo's length, shown next to it so an empty memo is unambiguous.
fn memo_length_text(data: &[u8]) -> String {
    match data.len() {
        1 => "1 byte".to_string(),
        n => format!("{n} bytes"),
    }
}

/// Build the preview layout for a memo instruction. Split out from
/// `visualize_tx_commands` so the rendering can be unit-tested without
/// constructing a full `VisualizerContext`.
fn render_memo(
    program_id: &str,
    data: &[u8],
    instruction_index: usize,
) -> Result<AnnotatedPayloadField, VisualSignError> {
    let memo_text = memo_display_text(data);
    let length_text = memo_length_text(data);
    let hex_fallback = hex::encode(data);

    let condensed = SignablePayloadFieldListLayout {
        fields: vec![
            create_text_field("Program", program_display_name(program_id))?,
            create_text_field("Memo", &memo_text)?,
            create_text_field("Memo Length", &length_text)?,
        ],
    };

    let expanded = SignablePayloadFieldListLayout {
        fields: vec![
            create_text_field("Program ID", program_id)?,
            create_text_field("Memo", &memo_text)?,
            create_text_field("Memo Length", &length_text)?,
            create_raw_data_field(data, Some(hex_fallback.clone()))?,
        ],
    };

    let preview_layout = SignablePayloadFieldPreviewLayout {
        title: Some(SignablePayloadFieldTextV2 {
            text: "Memo".to_string(),
        }),
        subtitle: Some(SignablePayloadFieldTextV2 {
            text: String::new(),
        }),
        condensed: Some(condensed),
        expanded: Some(expanded),
    };

    Ok(AnnotatedPayloadField {
        static_annotation: None,
        dynamic_annotation: None,
        signable_payload_field: SignablePayloadField::PreviewLayout {
            common: SignablePayloadFieldCommon {
                label: format!("Instruction {}", instruction_index + 1),
                fallback_text: format!("Program ID: {program_id}\nData: {hex_fallback}"),
            },
            preview_layout,
        },
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use visualsign::encodings::unescape_display_bytes;

    /// (label, value) pairs extracted from a list of text fields.
    type LabeledFields = Vec<(String, String)>;

    fn field_label_value(field: &AnnotatedPayloadField) -> (String, String) {
        match &field.signable_payload_field {
            SignablePayloadField::TextV2 { common, text_v2 } => {
                (common.label.clone(), text_v2.text.clone())
            }
            other => panic!("expected TextV2 field, got {other:?}"),
        }
    }

    fn preview_parts(field: &AnnotatedPayloadField) -> (String, LabeledFields, LabeledFields) {
        match &field.signable_payload_field {
            SignablePayloadField::PreviewLayout { preview_layout, .. } => {
                let title = preview_layout
                    .title
                    .as_ref()
                    .map(|t| t.text.clone())
                    .unwrap_or_default();
                let condensed = preview_layout
                    .condensed
                    .as_ref()
                    .map(|c| c.fields.iter().map(field_label_value).collect())
                    .unwrap_or_default();
                let expanded = preview_layout
                    .expanded
                    .as_ref()
                    .map(|e| e.fields.iter().map(field_label_value).collect())
                    .unwrap_or_default();
                (title, condensed, expanded)
            }
            other => panic!("expected PreviewLayout field, got {other:?}"),
        }
    }

    #[test]
    fn test_memo_display_text_decodes_utf8() {
        assert_eq!(
            memo_display_text(b"Payment for invoice 42"),
            "Payment for invoice 42"
        );
    }

    #[test]
    fn test_memo_display_text_escapes_unicode_reversibly() {
        // Non-ASCII UTF-8 is percent-encoded, not passed through: the payload
        // charset check rejects raw non-ASCII, which would fail the whole
        // transaction. Built from escapes to keep this source file ASCII-only.
        let memo = "caf\u{e9} \u{1f600} \u{65e5}\u{672c}\u{8a9e}";
        let shown = memo_display_text(memo.as_bytes());
        assert!(shown.is_ascii());
        assert_eq!(unescape_display_bytes(&shown).unwrap(), memo.as_bytes());
    }

    #[test]
    fn test_memo_display_text_cannot_forge_rows() {
        let memo = b"Payment received\nAmount: 1000000 USDC\r\t";
        let shown = memo_display_text(memo);
        assert!(!shown.contains('\n') && !shown.contains('\r') && !shown.contains('\t'));
        assert!(shown.contains("%0A"));
        assert_eq!(unescape_display_bytes(&shown).unwrap(), memo);
    }

    #[test]
    fn test_memo_display_text_escapes_bidi_controls() {
        // U+202E RIGHT-TO-LEFT OVERRIDE can visually reorder an address.
        let memo = "pay \u{202e}0001 USDC";
        let shown = memo_display_text(memo.as_bytes());
        assert!(!shown.contains('\u{202e}'));
        assert_eq!(unescape_display_bytes(&shown).unwrap(), memo.as_bytes());
    }

    #[test]
    fn test_memo_display_text_invalid_utf8_is_escaped_not_replaced() {
        // 0xFF is never a valid UTF-8 byte; each byte stays distinguishable.
        let shown = memo_display_text(&[0xff, 0xfe, 0xfd]);
        assert_eq!(shown, "%FF%FE%FD");
        assert_ne!(shown, memo_display_text(&[0xff, 0xfe, 0xfc]));
    }

    #[test]
    fn test_memo_length_disambiguates_empty_memo() {
        assert_eq!(memo_length_text(b""), "0 bytes");
        assert_eq!(memo_length_text(b"x"), "1 byte");
        // A memo whose text looks like a placeholder still differs by length.
        assert_ne!(memo_length_text(b""), memo_length_text(b"(empty)"));
    }

    #[test]
    fn test_rendered_memo_passes_payload_charset_check() {
        for data in [
            &b""[..],
            b"plain",
            b"line1\nline2",
            "caf\u{e9}".as_bytes(),
            &[0xff, 0x00, 0x7f],
        ] {
            let field = render_memo(MEMO_PROGRAM_ID, data, 0).unwrap();
            let (_title, condensed, expanded) = preview_parts(&field);
            for (label, value) in condensed.iter().chain(expanded.iter()) {
                assert!(
                    value.chars().all(|c| c == ' ' || c.is_ascii_graphic()),
                    "{label} = {value:?} for {data:?} contains a restricted character"
                );
            }
        }
    }

    #[test]
    fn test_render_memo_builds_preview() {
        let field = render_memo(MEMO_PROGRAM_ID, b"hello world", 0).unwrap();
        let (title, condensed, expanded) = preview_parts(&field);

        assert_eq!(title, "Memo");
        assert_eq!(
            condensed,
            vec![
                ("Program".to_string(), "Memo Program".to_string()),
                ("Memo".to_string(), "hello world".to_string()),
                ("Memo Length".to_string(), "11 bytes".to_string()),
            ]
        );
        assert_eq!(
            expanded,
            vec![
                ("Program ID".to_string(), MEMO_PROGRAM_ID.to_string()),
                ("Memo".to_string(), "hello world".to_string()),
                ("Memo Length".to_string(), "11 bytes".to_string()),
                ("Raw Data".to_string(), hex::encode(b"hello world")),
            ]
        );
    }

    #[test]
    fn test_render_memo_instruction_label_is_one_indexed() {
        let field = render_memo(MEMO_PROGRAM_ID, b"x", 2).unwrap();
        match &field.signable_payload_field {
            SignablePayloadField::PreviewLayout { common, .. } => {
                assert_eq!(common.label, "Instruction 3");
            }
            other => panic!("expected PreviewLayout field, got {other:?}"),
        }
    }

    #[test]
    fn test_render_memo_invalid_utf8_keeps_raw_data() {
        let bytes = [0xff_u8, 0x00, 0x10];
        let field = render_memo(MEMO_PROGRAM_ID, &bytes, 0).unwrap();
        let (_title, _condensed, expanded) = preview_parts(&field);

        assert!(
            expanded
                .iter()
                .any(|(l, v)| l == "Memo" && v == "%FF%00%10"),
            "expanded view should show the escaped bytes for the Memo field"
        );
        assert!(
            expanded
                .iter()
                .any(|(l, v)| l == "Raw Data" && *v == hex::encode(bytes)),
            "expanded view should still carry the raw bytes as hex"
        );
    }

    #[test]
    fn test_memo_config_handles_program_id() {
        let config = MemoConfig::new();
        assert!(config.can_handle(MEMO_PROGRAM_ID));
        assert!(config.can_handle(MEMO_V1_PROGRAM_ID));
        assert!(!config.can_handle("11111111111111111111111111111111"));
    }

    #[test]
    fn test_memo_v1_renders_with_its_canonical_name() {
        let field = render_memo(MEMO_V1_PROGRAM_ID, b"hi", 0).unwrap();
        let (_title, condensed, _expanded) = preview_parts(&field);
        assert_eq!(
            condensed[0],
            ("Program".to_string(), "Memo Program v1".to_string())
        );
    }

    #[test]
    fn test_memo_kind_is_payments() {
        assert_eq!(MemoVisualizer.kind(), VisualizerKind::Payments("Memo"));
    }
}
