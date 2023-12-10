#[derive(Debug, Clone, Default)]
pub struct Options {
    pub log: bool,
    pub skip_pgp_check: bool,
    pub skip_checksums: bool,
    pub no_prepare: bool,
    pub reproducable: bool,
    pub ignore_arch: bool,
    pub clean_build: bool,
    pub no_extract: bool,
    pub verify_source: bool,
    pub repackage: bool,
    pub no_build: bool,
    pub force: bool,
    pub check: bool,
    pub no_check: bool,
    pub no_archive: bool,
    pub holdver: bool,
}
