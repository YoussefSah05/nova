//! Local HLS reverse proxy built on Axum.
//!
//! Implemented in Phase 5: binds to 127.0.0.1 on an OS-assigned port,
//! rewrites upstream manifests to proxy all segment URLs, injects
//! subtitle tracks for source language and translations, and serves
//! rolling VTT files for live captions.
//! See docs/reference/hls-manifest.md for manifest rewriting rules.

#[cfg(test)]
mod tests {
    #[test]
    fn it_compiles() {}
}
