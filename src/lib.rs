#![allow(clippy::result_large_err)]

use std::fmt::Display;

pub use callback::*;
pub use makepkg::*;
pub use options::*;
use pkgbuild::Pkgbuild;

mod build;
mod callback;
mod fs;
mod integ;
mod lint_config;
mod lint_pkgbuild;
mod makepkg;
mod options;
mod package;
mod pacman;
mod raw;
mod run;
mod sources;
mod srcinfo;
mod util;

pub mod config;
pub mod error;
mod installation_variables;
pub mod pkgbuild;

pub(crate) static TOOL_NAME: &str = env!("CARGO_PKG_NAME");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    Pkgbuild,
    Config,
}

impl Display for FileKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FileKind::Pkgbuild => f.write_str(Pkgbuild::file_name()),
            FileKind::Config => todo!("config"),
        }
    }
}
