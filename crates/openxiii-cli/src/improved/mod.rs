//! Additional CLI commands registered by the improved edition.
use std::path::Path;

pub(super) fn command(_name: &str, _game: &Path, _args: &[String]) -> Option<Result<(), String>> {
    None
}
