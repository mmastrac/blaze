#[allow(dead_code)]
mod host;
pub mod machine;

#[cfg(all(feature = "pc-trace", not(target_arch = "wasm32")))]
pub mod pc_trace;
