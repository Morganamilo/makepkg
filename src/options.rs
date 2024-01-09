#[derive(Debug, Clone, Default)]
pub struct Options {
    pub log: bool,
    pub skip_pgp_check: bool,
    pub skip_checksums: bool,
    pub no_prepare: bool,
    pub reproducible: bool,
    pub ignore_arch: bool,
    pub clean_build: bool,
    pub no_extract: bool,
    pub verify_source: bool,
    pub repackage: bool,
    pub no_build: bool,
    pub recreate_package: bool,
    pub check: bool,
    pub no_check: bool,
    pub no_archive: bool,
    pub hold_ver: bool,
    pub all_source: bool,
}

impl Options {
    pub fn new() -> Self {
        Self::default()
    }
}
