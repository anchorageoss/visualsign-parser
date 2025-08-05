use sui_json_rpc_types::SuiArgument;
use sui_json_rpc_types::SuiArgument::Input;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Coin {
    pub id: String,
    pub label: String,
}

impl std::str::FromStr for Coin {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.trim().is_empty() {
            return Ok(Coin::default());
        }

        let mut parts = s.splitn(3, "::");
        let (id, label) = match (parts.next(), parts.next(), parts.next()) {
            (Some(id), _, Some(label)) => (id.to_string(), label.to_string()),
            (Some(id), _, None) => (id.to_string(), String::new()),
            _ => (String::new(), String::new()),
        };

        Ok(Coin { id, label })
    }
}

impl Coin {
    pub fn label(&self) -> &str {
        &self.label
    }
}

impl std::fmt::Display for Coin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}::{}", self.id, self.label)
    }
}

impl Default for Coin {
    fn default() -> Self {
        Coin {
            id: "0x0".to_string(),
            label: "Unknown".to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum CoinObject {
    #[allow(dead_code)]
    Sui,
    Unknown(String),
}

impl std::fmt::Display for CoinObject {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CoinObject::Sui => write!(f, "Sui"),
            CoinObject::Unknown(s) => write!(f, "Object ID: {}", s),
        }
    }
}

impl CoinObject {
    pub fn get_label(&self) -> String {
        match self {
            CoinObject::Sui => "Sui".to_string(),
            CoinObject::Unknown(_) => "Unknown".to_string(),
        }
    }
}

impl Default for CoinObject {
    fn default() -> CoinObject {
        CoinObject::Unknown(String::default())
    }
}

/// Get index from SUI arguments array (expects single argument)
pub fn get_index(sui_args: &[SuiArgument], index: Option<usize>) -> Option<u16> {
    let arg = match index {
        Some(i) => sui_args.get(i)?,
        None => sui_args.first()?,
    };

    parse_numeric_argument(arg)
}

/// Parse numeric argument from SUI argument (Input or Result)
pub fn parse_numeric_argument(arg: &SuiArgument) -> Option<u16> {
    match arg {
        Input(index) => Some(*index),
        SuiArgument::Result(index) => Some(*index),
        _ => None,
    }
}
