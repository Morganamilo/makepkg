#[derive(Debug, Clone, Default)]
pub struct Options {
    pub no_deps: bool,
    pub sync_deps: bool,
    pub install: bool,
    pub log: bool,

    pub clean: bool,
    pub clean_build: bool,
    pub ignore_arch: bool,
    pub hold_ver: bool,

    pub no_download: bool,
    pub no_checksums: bool,
    pub no_signatures: bool,
    pub no_verify: bool,
    pub no_extract: bool,
    pub no_prepare: bool,
    pub no_build: bool,
    pub keep_pkg: bool,
    pub no_check: bool,
    pub no_package: bool,
    pub no_archive: bool,
    pub rebuild: bool,
}

impl Options {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn no_build(&mut self) {
        self.no_build = true;
        self.no_check = true;
        self.no_package = true;
        self.no_archive = true;
    }

    pub fn verify_source(&mut self) {
        self.no_build();
        self.no_extract = true;
        self.no_prepare = true;
    }

    pub fn repackage(&mut self) {
        self.no_integ();
        self.no_download = true;
        self.no_extract = true;
        self.no_prepare = true;
        self.no_verify = true;
        self.no_build = true;
        self.no_check = true;
        self.rebuild = true;
    }

    pub fn no_integ(&mut self) {
        self.no_signatures = true;
        self.no_checksums = true;
    }
}
