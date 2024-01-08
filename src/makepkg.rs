use std::{cell::RefCell, process::Child};

use crate::{
    callback::CallBacks,
    config::{Config, PkgbuildDirs},
    error::Result,
    pkgbuild::Pkgbuild,
};

#[derive(Debug)]
pub(crate) struct FakeRoot {
    pub child: Child,
    pub key: String,
}

impl Drop for FakeRoot {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

#[derive(Debug, Default)]
pub struct Makepkg {
    pub config: Config,
    pub(crate) callbacks: Option<Box<RefCell<dyn CallBacks>>>,
    pub(crate) fakeroot: RefCell<Option<FakeRoot>>,
}

impl Makepkg {
    pub fn new() -> Result<Makepkg> {
        let config = Config::new()?;

        let makepkg = Makepkg {
            config,
            ..Makepkg::default()
        };

        Ok(makepkg)
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    pub fn pkgbuild_dirs(&self, pkgbuild: &Pkgbuild) -> Result<PkgbuildDirs> {
        self.config.pkgbuild_dirs(pkgbuild)
    }
}
