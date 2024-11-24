pub mod composition;
pub mod fixtures;
pub mod import;
pub mod nar;
pub mod path_info;
pub mod pathinfoservice;
pub mod proto;
pub mod utils;

#[cfg(test)]
mod tests;

// Used as user agent in various HTTP Clients
const USER_AGENT: &str = concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"));

// That's what the rstest_reuse README asks us do, and fails about being unable
// to find rstest_reuse in crate root.
#[cfg(test)]
#[allow(clippy::single_component_path_imports)]
use rstest_reuse;
