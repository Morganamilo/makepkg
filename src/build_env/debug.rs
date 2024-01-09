use std::{collections::BTreeMap, ffi::OsString};

use crate::{config::PkgbuildDirs, pkgbuild::Pkgbuild, Makepkg};

impl Makepkg {
    pub(crate) fn debug_flags(
        &self,
        dirs: &PkgbuildDirs,
        pkgbuild: &Pkgbuild,
        envs: &mut BTreeMap<String, OsString>,
    ) {
        let config = &self.config;

        if config.option(pkgbuild, "debug").enabled()
            && !config.option(pkgbuild, "buildflags").disabled()
        {
            let remap = format!(
                " -ffile-prefix-map={}={}/{}",
                dirs.srcdir.display(),
                self.config.dbg_srcdir.display(),
                pkgbuild.pkgbase,
            );

            let rust_remap = format!(
                " --remap-path-prefix={}={}/{}",
                dirs.srcdir.display(),
                self.config.dbg_srcdir.display(),
                pkgbuild.pkgbase,
            );

            let debug_flags = envs.entry("DEBUG_CFLAGS".into()).or_default();
            debug_flags.push(&remap);
            let debug_flags = debug_flags.clone();
            let flags = envs.entry("CFLAGS".into()).or_default();
            flags.push(" ");
            flags.push(debug_flags);

            let debug_flags = envs.entry("DEBUG_CXXFLAGS".into()).or_default();
            debug_flags.push(&remap);
            let debug_flags = debug_flags.clone();
            let flags = envs.entry("CXXFLAGS".into()).or_default();
            flags.push(" ");
            flags.push(debug_flags);

            let debug_flags = envs.entry("DEBUG_RUSTFLAGS".into()).or_default();
            debug_flags.push(&rust_remap);
            let debug_flags = debug_flags.clone();
            let flags = envs.entry("RUSTFLAGS".into()).or_default();
            flags.push(" ");
            flags.push(debug_flags);
        }
    }
}
