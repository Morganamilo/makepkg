mod buildflags;
mod compiler;
mod debug;

use std::{collections::BTreeMap, ffi::OsString, process::Command};

use crate::{config::PkgbuildDirs, pkgbuild::Pkgbuild, Makepkg};

impl Makepkg {
    pub(crate) fn build_env(
        &self,
        dirs: &PkgbuildDirs,
        pkgbuild: &Pkgbuild,
        command: &mut Command,
    ) {
        let env = self.generate_build_env(dirs, pkgbuild);
        for (k, v) in env {
            command.env(k, v);
        }
    }

    fn generate_build_env(
        &self,
        dirs: &PkgbuildDirs,
        pkgbuild: &Pkgbuild,
    ) -> BTreeMap<String, OsString> {
        let mut env = BTreeMap::new();
        self.compiler(dirs, pkgbuild, &mut env);
        self.build_flags(dirs, pkgbuild, &mut env);
        self.debug_flags(dirs, pkgbuild, &mut env);
        env
    }
}
