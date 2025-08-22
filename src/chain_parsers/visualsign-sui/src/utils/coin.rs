#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuiCoin {
    pub address: String,
    pub name: String,
    pub symbol: String,
}

impl std::str::FromStr for SuiCoin {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.trim().is_empty() {
            return Ok(SuiCoin::default());
        }

        let mut parts = s.splitn(3, "::");
        let (address, name, symbol) = match (parts.next(), parts.next(), parts.next()) {
            (Some(address), Some(name), Some(symbol)) => {
                (address.to_string(), name.to_string(), symbol.to_string())
            }
            _ => (String::new(), String::new(), String::new()),
        };

        Ok(SuiCoin {
            address,
            name,
            symbol,
        })
    }
}

impl SuiCoin {
    pub fn symbol(&self) -> &str {
        &self.symbol
    }
}

impl std::fmt::Display for SuiCoin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}::{}::{}", self.address, self.name, self.symbol)
    }
}

impl Default for SuiCoin {
    fn default() -> Self {
        SuiCoin {
            address: "0x0".to_string(),
            name: "Unknown".to_string(),
            symbol: "Unknown".to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum CoinObject {
    Sui,
    UnknownObject(String),
}

impl std::fmt::Display for CoinObject {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CoinObject::Sui => write!(f, "Sui"),
            CoinObject::UnknownObject(s) => write!(f, "Object ID: {}", s),
        }
    }
}

impl CoinObject {
    pub fn get_label(&self) -> String {
        match self {
            CoinObject::Sui => "MIST".to_string(),
            CoinObject::UnknownObject(_) => "Unknown".to_string(),
        }
    }
}

impl Default for CoinObject {
    fn default() -> CoinObject {
        CoinObject::UnknownObject(String::default())
    }
}
