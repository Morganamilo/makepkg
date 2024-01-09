use std::{collections::BTreeMap, ffi::OsString, path::Path};

use crate::{config::PkgbuildDirs, installation_variables::LIBDIR, pkgbuild::Pkgbuild, Makepkg};

impl Makepkg {
    pub(crate) fn compiler(
        &self,
        dirs: &PkgbuildDirs,
        pkgbuild: &Pkgbuild,
        envs: &mut BTreeMap<String, OsString>,
    ) {
        let bin = Path::new(LIBDIR).join("ccache/bin");
        let mut using_ccache = false;
        let config = &self.config;

        if config.build_option(pkgbuild, "ccache").enabled() && bin.exists() {
            let path = env("PATH", envs);
            let mut newpath = bin.into_os_string();
            using_ccache = true;
            newpath.push(":");
            newpath.push(&path);
            *path = newpath;
        }

        if config.build_option(pkgbuild, "distcc").enabled() {
            if using_ccache {
                let prefix = env("CCACHE_PREFIX", envs);
                if !prefix.to_string_lossy().contains(" distcc ") {
                    prefix.push(" distcc");
                }
                envs.insert("CCACHE_BASEDIR".into(), dirs.srcdir.clone().into());
            } else {
                let mut newpath = Path::new(LIBDIR).join("distcc/bin");
                if newpath.exists() {
                    let path = env("PATH", envs);
                    newpath.push(":");
                    newpath.push(&path);
                    *path = newpath.into();
                }
            }
            envs.insert(
                "DISTCC_HOSTS".into(),
                self.config.distcc_hosts.clone().into(),
            );
        }
    }
}

fn env<'a>(var: &str, env: &'a mut BTreeMap<String, OsString>) -> &'a mut OsString {
    let cur_env = std::env::var_os(var).unwrap_or_default();
    env.entry(var.into()).or_insert(cur_env)
}
