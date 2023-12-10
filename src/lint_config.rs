use std::fmt::Display;

use crate::{
    config::Config,
    error::LintKind,
    raw::{RawConfig, Value, Variable},
};

#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub enum Warning {
    InvalidPackager(String),
}

impl Display for Warning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Warning::InvalidPackager(_) => write!(
                f,
                "PACKAGER should have the format 'Example Name <email@address.invalid>'"
            ),
        }
    }
}

impl RawConfig {
    pub(crate) fn lint(&self, lints: &mut Vec<LintKind>) {
        lint_arrays(self.variables.iter(), lints);
        lint_newline(self.variables.iter(), lints);
    }
}

impl Config {
    pub fn warnings(&self) -> Vec<Warning> {
        let mut warnings = Vec::new();
        warn_packager(self, &mut warnings);

        warnings
    }

    pub(crate) fn lint(&self, lints: &mut Vec<LintKind>) {
        lint_ext(self, lints);
    }
}

fn warn_packager(config: &Config, warnings: &mut Vec<Warning>) {
    if config.packager == "Unknown Packager" {
        return;
    }

    if !config.packager.contains(char::is_alphabetic)
        || ![' ', '<', '@', '>']
            .iter()
            .all(|c| config.packager.contains(*c))
    {
        warnings.push(Warning::InvalidPackager(config.packager.clone()))
    }
}

fn lint_ext(config: &Config, lints: &mut Vec<LintKind>) {
    if !config.pkgext.starts_with(".pkg.tar") {
        lints.push(LintKind::InvalidPkgExt(config.pkgext.clone()))
    }
    if !config.srcext.starts_with(".src.tar") {
        lints.push(LintKind::InvalidSrcExt(config.srcext.clone()))
    }
}

fn lint_newline<'a, I: Iterator<Item = &'a Variable>>(iter: I, lints: &mut Vec<LintKind>) {
    for var in iter {
        match &var.value {
            Value::Array(a) => {
                if a.iter().any(|v| v.contains('\n')) {
                    lints.push(LintKind::VariabeContainsNewlines(var.name.clone()))
                }
            }
            Value::String(s) => {
                if s.contains('\n') {
                    lints.push(LintKind::VariabeContainsNewlines(var.name.clone()))
                }
            }
            _ => (),
        }
    }
}

fn lint_arrays<'a, I: Iterator<Item = &'a Variable>>(iter: I, lints: &mut Vec<LintKind>) {
    for var in iter {
        match &var.value {
            Value::Array(a) => {
                if a.iter().any(|v| v.is_empty()) {
                    lints.push(LintKind::VariabeContainsEmptyString(var.name.clone()))
                }
            }
            _ => (),
        }
    }
}
