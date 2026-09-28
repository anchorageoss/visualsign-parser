// `RpcClient` returns the SDK's large `ClientError` by value throughout this
// crate's helpers; we can't shrink it.
#![allow(clippy::result_large_err)]

pub mod surfpool;

pub use surfpool::{SurfpoolConfig, SurfpoolManager};
