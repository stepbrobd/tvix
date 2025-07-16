pub mod builtins;
// pub mod fetchers;
pub mod known_paths;
// pub mod tvix_build;
pub mod tvix_io;
pub mod tvix_store_io;

// mod fetchurl;

// Used as user agent in various HTTP Clients
#[allow(dead_code)]
const USER_AGENT: &str = concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"));

#[cfg(test)]
mod tests;

/// Tell the Evaluator to resolve `<nix>` to the path `/__corepkgs__`,
/// which has special handling in [tvix_io::TvixIO].
/// This is used in nixpkgs to import `fetchurl.nix` from `<nix>`.
pub fn configure_nix_path<'co, 'ro, 'env>(
    eval_builder: tvix_eval::EvaluationBuilder<'co, 'ro, 'env>,
    nix_search_path: &Option<String>,
) -> tvix_eval::EvaluationBuilder<'co, 'ro, 'env> {
    eval_builder.nix_path(
        nix_search_path
            .as_ref()
            .map(|p| format!("nix=/__corepkgs__:{p}"))
            .or_else(|| Some("nix=/__corepkgs__".to_string())),
    )
}
