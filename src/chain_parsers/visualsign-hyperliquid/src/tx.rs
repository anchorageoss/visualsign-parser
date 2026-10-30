//! The VisualSign input type and converter.

use visualsign::vsptrait::{
    ConversionResult, Transaction, TransactionParseError, VisualSignConverter,
    VisualSignConverterFromString, VisualSignError, VisualSignOptions,
};

use crate::input::{self, L1Request, Request, UserSignedRequest};
use crate::render;

/// A verified Hyperliquid signing request.
#[derive(Debug, Clone)]
pub enum HyperliquidTransaction {
    /// An L1 action whose `connectionId` matched the action bytes.
    L1(Box<L1Request>),
    /// A user-signed action with a pinned domain and type.
    UserSigned(UserSignedRequest),
}

impl Transaction for HyperliquidTransaction {
    fn from_string(data: &str) -> Result<Self, TransactionParseError> {
        match input::parse(data).map_err(TransactionParseError::DecodeError)? {
            Request::L1(req) => Ok(Self::L1(req)),
            Request::UserSigned(req) => Ok(Self::UserSigned(req)),
        }
    }

    fn transaction_type(&self) -> String {
        match self {
            Self::L1(_) => "Hyperliquid L1 Action".to_string(),
            Self::UserSigned(_) => "Hyperliquid User-Signed Action".to_string(),
        }
    }
}

/// Converter for [`HyperliquidTransaction`].
#[derive(Debug, Clone, Copy, Default)]
pub struct HyperliquidVisualSignConverter;

impl VisualSignConverter<HyperliquidTransaction> for HyperliquidVisualSignConverter {
    fn to_visual_sign_payload(
        &self,
        transaction: HyperliquidTransaction,
        options: VisualSignOptions,
    ) -> Result<ConversionResult, VisualSignError> {
        let payload = match &transaction {
            HyperliquidTransaction::L1(req) => render::l1_payload(req, options.transaction_name)?,
            HyperliquidTransaction::UserSigned(req) => {
                render::user_signed_payload(req, options.transaction_name)?
            }
        };
        Ok(ConversionResult::new(payload))
    }
}

impl VisualSignConverterFromString<HyperliquidTransaction> for HyperliquidVisualSignConverter {}
