pub mod google {
    pub mod rpc {
        include!("google.rpc.rs");
    }
}
pub mod health {
    include!("health.rs");
}
pub mod parser {
    include!("parser.rs");
}
