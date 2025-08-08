#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuiPackage {
    pub address: String,
    pub module: String,
    pub resource: String,
}

impl std::str::FromStr for SuiPackage {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.trim().is_empty() {
            return Ok(SuiPackage::default());
        }

        let mut parts = s.splitn(3, "::");
        let (address, module, resource) = match (parts.next(), parts.next(), parts.next()) {
            (Some(address), Some(module), Some(resource)) => (
                address.to_string(),
                module.to_string(),
                resource.to_string(),
            ),
            _ => (String::new(), String::new(), String::new()),
        };

        Ok(SuiPackage {
            address,
            module,
            resource,
        })
    }
}

impl std::fmt::Display for SuiPackage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}::{}::{}", self.address, self.module, self.resource)
    }
}

impl Default for SuiPackage {
    fn default() -> Self {
        SuiPackage {
            address: "0x0".to_string(),
            module: "Unknown".to_string(),
            resource: "Unknown".to_string(),
        }
    }
}
