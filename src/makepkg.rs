use std::{cell::RefCell, process::Child};

use crate::{
    callback::Callbacks,
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

impl FakeRoot {
    pub(crate) fn library_name() -> &'static str {
        if cfg!(target_vendor = "apple") {
            "libfakeroot.dylib"
        } else {
            "libfakeroot.so"
        }
    }
}

#[derive(Debug)]
pub struct Makepkg {
    pub config: Config,
    pub(crate) callbacks: RefCell<Option<Box<dyn Callbacks>>>,
    pub(crate) fakeroot: RefCell<Option<FakeRoot>>,
    pub(crate) id: RefCell<usize>,
}

impl Makepkg {
    pub fn new() -> Result<Makepkg> {
        let config = Config::new()?;
        Ok(Self::from_config(config))
    }

    pub fn from_config(config: Config) -> Makepkg {
        Makepkg {
            config,
            callbacks: RefCell::new(None),
            fakeroot: RefCell::new(None),
            id: RefCell::new(0),
        }
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    pub fn pkgbuild_dirs(&self, pkgbuild: &Pkgbuild) -> Result<PkgbuildDirs> {
        self.config.pkgbuild_dirs(pkgbuild)
    }

    pub fn callbacks<CB: Callbacks>(mut self, callbacks: CB) -> Self {
        self.callbacks = RefCell::new(Some(Box::new(callbacks)));
        self
    }
}
