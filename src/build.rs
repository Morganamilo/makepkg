use nix::sys::stat::{umask, Mode};

use crate::{
    callback::Event,
    error::{AlreadyBuiltError, ArchitectureError, Context, Result},
    fs::{mkdir, rm_all},
    options::Options,
    package::PackageKind,
    pkgbuild::{Function, Pkgbuild},
    Makepkg,
};

impl Makepkg {
    pub fn build(&self, options: &Options, pkgbuild: &mut Pkgbuild) -> Result<()> {
        umask(Mode::from_bits_truncate(0o022));

        self.event(Event::BuildingPackage(
            pkgbuild.pkgbase.clone(),
            pkgbuild.version(),
        ));

        let config = &self.config;

        if !options.ignore_arch && !self.arch_supported(pkgbuild) {
            return Err(ArchitectureError {
                pkgbase: pkgbuild.pkgbase.clone(),
                arch: config.arch.clone(),
            }
            .into());
        }

        if !pkgbuild.has_function(Function::Pkgver) {
            self.err_if_built(options, pkgbuild)?;
        }

        let dirs = self.pkgbuild_dirs(pkgbuild)?;

        if !options.repackage {
            if options.no_extract && !options.verify_source {
                self.event(Event::UsingExistingSrcdir);
            } else {
                self.download_sources(options, pkgbuild, false)?;
                self.check_integ(options, pkgbuild, false)?;

                if options.verify_source {
                    return Ok(());
                }

                if options.clean_build && dirs.srcdir.exists() {
                    self.event(Event::RemovingSrcdir);
                    rm_all(&dirs.srcdir, Context::BuildPackage)?;
                }
                mkdir(&dirs.srcdir, Context::BuildPackage)?;

                self.extract_sources(options, pkgbuild, false)?;
                self.update_pkgver(options, pkgbuild)?;
                self.err_if_built(options, pkgbuild)?;
            }
        }

        if options.no_build {
            return Ok(());
        }

        if dirs.pkgdir.exists() {
            self.event(Event::RemovingPkgdir);
            rm_all(&dirs.pkgdir, Context::BuildPackage)?;
        }
        for pkg in pkgbuild.packages() {
            mkdir(&dirs.pkgdir(pkg), Context::BuildPackage)?;
        }

        if !options.repackage {
            self.run_function(options, pkgbuild, Function::Build)?;
            if config.option(pkgbuild, "check").enabled()
                || (config.build_option(pkgbuild, "check").enabled() && !options.check.disabled())
            {
                self.run_function(options, pkgbuild, Function::Check)?;
            }
        }

        self.run_function(options, pkgbuild, Function::Package)?;

        if !options.no_archive {
            for pkg in pkgbuild.packages() {
                self.create_package(&dirs, options, pkgbuild, pkg, false)?;
            }
        }

        self.event(Event::BuiltPackage(
            pkgbuild.pkgbase.clone(),
            pkgbuild.version(),
        ));

        Ok(())
    }

    pub fn arch_supported(&self, pkgbuild: &Pkgbuild) -> bool {
        pkgbuild
            .arch
            .iter()
            .any(|a| *a == self.config.arch || a == "any")
    }

    pub fn is_srcpkg_built(&self, pkgbuild: &Pkgbuild) -> Result<bool> {
        let dirs = self.pkgbuild_dirs(pkgbuild)?;
        let ver = pkgbuild.version();
        let name = format!("{}-{}{}", pkgbuild.pkgbase, ver, self.config.srcext);
        let path = dirs.pkgdest.join(name);
        Ok(path.exists())
    }

    pub fn is_pkg_built(&self, pkgbuild: &Pkgbuild) -> Result<bool> {
        let dirs = self.pkgbuild_dirs(pkgbuild)?;
        let ver = pkgbuild.version();

        for pkg in pkgbuild.pkgnames() {
            let name = format!("{}-{}-{}{}", pkg, ver, self.config.arch, self.config.pkgext);
            let path = dirs.pkgdest.join(name);

            if !path.exists() {
                return Ok(false);
            }
        }

        Ok(true)
    }

    pub fn err_if_srcpkg_built(&self, options: &Options, pkgbuild: &Pkgbuild) -> Result<()> {
        if !options.rebuild && self.is_srcpkg_built(pkgbuild)? {
            return Err(AlreadyBuiltError {
                kind: PackageKind::Source,
                pkgbase: pkgbuild.pkgbase.clone(),
            }
            .into());
        }
        Ok(())
    }
    pub fn err_if_built(&self, options: &Options, pkgbuild: &Pkgbuild) -> Result<()> {
        if !options.rebuild && self.is_pkg_built(pkgbuild)? {
            return Err(AlreadyBuiltError {
                kind: PackageKind::Package,
                pkgbase: pkgbuild.pkgbase.clone(),
            }
            .into());
        }
        Ok(())
    }
}
