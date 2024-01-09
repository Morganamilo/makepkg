use std::{collections::BTreeMap, ffi::OsString};

use crate::{config::PkgbuildDirs, pkgbuild::Pkgbuild, Makepkg};

impl Makepkg {
    pub(crate) fn build_flags(
        &self,
        _dirs: &PkgbuildDirs,
        pkgbuild: &Pkgbuild,
        envs: &mut BTreeMap<String, OsString>,
    ) {
        let config = &self.config;

        if !config.option(pkgbuild, "buildflags").disabled() {
            envs.insert("CFLAGS".into(), self.config.cflags.clone().into());
            envs.insert("CPPFLAGS".into(), self.config.cppflags.clone().into());
            envs.insert("CXXFLAGS".into(), self.config.cxxflags.clone().into());
            envs.insert("LDFLAGS".into(), self.config.ldflags.clone().into());
            envs.insert("CHOST".into(), self.config.chost.clone().into());

            if config.option(pkgbuild, "lto").enabled() {
                let flags = envs.entry("CFLAGS".into()).or_default();
                flags.push(" ");
                flags.push(&self.config.ltoflags);

                let flags = envs.entry("CXXFLAGS".into()).or_default();
                flags.push(" ");
                flags.push(&self.config.ltoflags);

                let flags = envs.entry("LDFLAGS".into()).or_default();
                flags.push(" ");
                flags.push(&self.config.ltoflags);
            }

            if !self.config.option(pkgbuild, "makeflags").disabled() {
                envs.insert("MAKEFLAGS".into(), self.config.makeflags.clone().into());
            }
        }
    }
}
