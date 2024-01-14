use std::{
    collections::HashSet,
    fmt::Display,
    fs::File,
    io::Write,
    os::{
        unix::fs::MetadataExt,
        unix::{ffi::OsStrExt, fs::PermissionsExt},
    },
    path::Path,
    process::{Command, Stdio},
};

use nix::{
    sys::stat::{umask, Mode},
    NixPath,
};
use sha2::Sha256;

use crate::{
    callback::{CommandKind, Event, LogLevel, LogMessage},
    config::PkgbuildDirs,
    error::{CommandErrorExt, CommandOutputExt, Context, IOContext, IOErrorExt, Result},
    fs::{copy, copy_dir, mkdir, open, rm_all, set_time, write},
    installation_variables::FAKEROOT_LIBDIRS,
    integ::hash_file,
    options::Options,
    pacman::buildinfo_installed,
    pkgbuild::{Package, Pkgbuild},
    run::CommandOutput,
    FakeRoot, Makepkg,
};

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum PackageKind {
    Package,
    Source,
}

impl Display for PackageKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PackageKind::Package => f.write_str("package"),
            PackageKind::Source => f.write_str("source package"),
        }
    }
}

impl Makepkg {
    pub(crate) fn create_package(
        &self,
        dirs: &PkgbuildDirs,
        options: &Options,
        pkgbuild: &Pkgbuild,
        pkg: &Package,
        debug: bool,
    ) -> Result<()> {
        if debug {
            self.event(Event::CreatingDebugPackage(&pkg.pkgname))?;
        } else {
            self.event(Event::CreatingPackage(&pkg.pkgname))?;
        }

        let pkgdir = dirs.pkgdir(pkg);

        self.generate_pkginfo(dirs, pkgbuild, pkg, debug)?;
        self.generate_buildinfo(dirs, pkgbuild, pkg)?;

        if let Some(install) = &pkg.install {
            let dest = pkgdir.join(".INSTALL");
            self.event(Event::AddingFileToPackage(install))?;
            let install = dirs.startdir.join(install);
            copy(install, &dest, Context::CreatePackage)?;
            std::fs::set_permissions(&dest, PermissionsExt::from_mode(0o644))
                .context(Context::CreatePackage, IOContext::Chmod(dest))?;
        }

        if let Some(changelog) = &pkg.changelog {
            self.event(Event::AddingFileToPackage(changelog))?;
            let changelog = dirs.startdir.join(changelog);
            let dest = pkgdir.join(".CHANGELOG");
            copy(changelog, &dest, Context::CreatePackage)?;
            std::fs::set_permissions(&dest, PermissionsExt::from_mode(0o644))
                .context(Context::CreatePackage, IOContext::Chmod(dest))?;
        }

        for file in walkdir::WalkDir::new(&pkgdir) {
            let file = file.context(Context::CreatePackage, IOContext::ReadDir(pkgdir.clone()))?;
            set_time(file.path(), self.config.source_date_epoch, false)?;
        }

        self.generate_mtree(dirs, pkgbuild, pkg)?;

        set_time(pkgdir.join(".MTREE"), self.config.source_date_epoch, false)?;

        if !options.no_archive {
            self.make_archive(dirs, pkgbuild, &pkgbuild.packages[0], false)?;
        }

        Ok(())
    }

    fn generate_mtree(
        &self,
        dirs: &PkgbuildDirs,
        pkgbuild: &Pkgbuild,
        pkg: &Package,
    ) -> Result<()> {
        self.event(Event::GeneratingPackageFile(".MTREE"))?;
        let pkgdir = dirs.pkgdir(pkg);
        let files = self.package_files(&pkgdir)?;

        let mtree = pkgdir.join(".MTREE");
        let mut file = File::options();
        file.create(true).write(true).truncate(true);
        let mtree = open(&file, mtree, Context::GeneratePackageFile(".MTREE".into()))?;

        let mut tarcmd = Command::new("bsdtar");
        self.fakeroot_env(&mut tarcmd)?;
        tarcmd
            .arg("-cnf")
            .arg("-")
            .arg("--format=mtree")
            .arg("--options=!all,use-set,type,uid,gid,mode,time,size,md5,sha256,link")
            .arg("--null")
            .arg("--files-from")
            .arg("-")
            .arg("--exclude")
            .arg(".MTREE")
            .env("LANG", "C")
            .current_dir(&pkgdir)
            .stdout(Stdio::piped())
            .stdin(Stdio::piped());

        let mut gzip = Command::new("gzip");
        gzip.arg("-cfn").stdout(mtree);

        tarcmd
            .process_pipe(
                self,
                CommandKind::BuildingPackage(pkgbuild),
                files.as_slice(),
                &mut gzip,
            )
            .cmd_context(&tarcmd, Context::GeneratePackageFile(".MTREE".into()))?;

        Ok(())
    }

    fn make_archive(
        &self,
        dirs: &PkgbuildDirs,
        pkgbuild: &Pkgbuild,
        pkg: &Package,
        srcpkg: bool,
    ) -> Result<()> {
        let pkgdir;
        let pkgname;
        let pkgfilename;
        let pkgfile;
        let compress;

        if srcpkg {
            pkgname = pkgbuild.pkgbase.as_str();
            pkgdir = dirs.srcpkgdir.parent().unwrap().to_path_buf();
            pkgfilename = format!("{}-{}{}", pkgname, pkgbuild.version(), self.config.srcext);
            pkgfile = dirs.srcpkgdest.join(&pkgfilename);
            compress = self.config.srcext.compress();
        } else {
            pkgname = pkg.pkgname.as_str();
            pkgdir = dirs.pkgdir(pkg);
            pkgfilename = format!(
                "{}-{}-{}{}",
                pkgname,
                pkgbuild.version(),
                self.config.arch,
                self.config.pkgext
            );
            pkgfile = dirs.srcpkgdest.join(&pkgfilename);
            compress = self.config.pkgext.compress();
        };

        let compress = self.config.compress_args(compress);
        let compress_prog = &compress[0];

        let create_flags = if srcpkg { "-cLf" } else { "-cnf" };

        let files = if srcpkg {
            Vec::new()
        } else {
            self.event(Event::GeneratingPackageFile(&pkgfilename))?;
            self.package_files(&pkgdir)?
        };

        let mut file = File::options();
        file.create(true).write(true).truncate(true);
        let pkgfile = open(&file, pkgfile, Context::CreatePackage)?;

        let mut tarcmd = Command::new("bsdtar");
        self.fakeroot_env(&mut tarcmd)?;

        tarcmd
            .arg("--no-fflags")
            .arg(create_flags)
            .arg("-")
            .env("LANG", "C")
            .stdout(Stdio::piped())
            .stdin(Stdio::piped());

        if srcpkg {
            tarcmd
                .current_dir(&pkgdir)
                .arg("-cLf")
                .arg("-")
                .arg(pkgname);
        } else {
            tarcmd
                .current_dir(&pkgdir)
                .arg("-cnf")
                .arg("--null")
                .arg("--files-from")
                .arg("-");
        }

        let mut zipcmd = Command::new(compress_prog);
        zipcmd.args(&compress[1..]).stdout(pkgfile);

        tarcmd
            .process_pipe(
                self,
                CommandKind::BuildingPackage(pkgbuild),
                files.as_slice(),
                &mut zipcmd,
            )
            .cmd_context(&tarcmd, Context::CreatePackage)?;

        Ok(())
    }

    fn generate_buildinfo(
        &self,
        dirs: &PkgbuildDirs,
        pkgbuild: &Pkgbuild,
        pkg: &Package,
    ) -> Result<()> {
        self.event(Event::GeneratingPackageFile(".BUILDINFO"))?;
        let binfo = dirs.pkgdir(pkg).join(".BUILDINFO");
        let mut file = File::options();
        file.write(true).create(true).truncate(true);
        let mut file = open(
            &file,
            &binfo,
            Context::GeneratePackageFile(".BUILDINFO".into()),
        )?;
        let c = self.config();

        let p = binfo.as_path();

        self.write_kv(p, &mut file, "format", "2")?;
        self.write_kv(p, &mut file, "pkgname", &pkg.pkgname)?;
        self.write_kv(p, &mut file, "pkgbase", &pkgbuild.pkgbase)?;
        self.write_kv(p, &mut file, "pkgver", &pkgbuild.version())?;
        self.write_kv(p, &mut file, "pkgarch", &c.arch)?;
        let hash = hash_file::<Sha256>(&dirs.pkgbuild)?;
        self.write_kv(p, &mut file, "pkgbuild_sha256sum", &hash)?;
        self.write_kv(p, &mut file, "packager", &c.packager)?;
        self.write_kv(p, &mut file, "builddate", &c.source_date_epoch.to_string())?;
        self.write_kv(
            p,
            &mut file,
            "builddir",
            &dirs.builddir.display().to_string(),
        )?;
        self.write_kv(
            p,
            &mut file,
            "startdir",
            &dirs.startdir.display().to_string(),
        )?;
        self.write_kv(p, &mut file, "buildtool", &c.buildtool)?;
        self.write_kv(p, &mut file, "buildtoolver", &c.buildtoolver)?;

        self.write_kvs(
            p,
            &mut file,
            "buildenv",
            c.build_env.values.iter().map(|s| s.to_string()),
        )?;
        self.write_kvs(
            p,
            &mut file,
            "options",
            c.options.values.iter().map(|s| s.to_string()),
        )?;

        let installed = buildinfo_installed(self, pkgbuild);

        //TODO warn no pacman installed
        if let Ok(installed) = installed {
            self.write_kvs(p, &mut file, "installed", installed)?;
        }
        Ok(())
    }

    fn generate_pkginfo(
        &self,
        dirs: &PkgbuildDirs,
        pkgbuild: &Pkgbuild,
        pkg: &Package,
        debug: bool,
    ) -> Result<()> {
        self.event(Event::GeneratingPackageFile(".PKGINFO"))?;

        let size = self.package_size(dirs, pkg)?;
        let c = self.config();

        let pkgdir = dirs.pkgdir(pkg).join(".PKGINFO");
        let mut file = File::options();
        file.write(true).create(true).truncate(true);
        let mut file = open(
            &file,
            &pkgdir,
            Context::GeneratePackageFile(".PKGINFO".into()),
        )?;

        let mut fakerootcmd = Command::new("fakeroot");
        let fakeroot = fakerootcmd
            .arg("-v")
            .process_read(self, CommandKind::BuildingPackage(pkgbuild))
            .read(
                &fakerootcmd,
                Context::GeneratePackageFile(".PKGINFO".into()),
            )?;

        writeln!(
            file,
            "# Generated by {} {}",
            self.config.buildtool, self.config.buildtoolver,
        )
        .context(
            Context::GeneratePackageFile(".PKGINFO".to_string()),
            IOContext::Write(pkgdir.clone()),
        )?;
        writeln!(file, "# using {}", fakeroot.trim()).context(
            Context::GeneratePackageFile(".PKGINFO".to_string()),
            IOContext::Write(pkgdir.clone()),
        )?;

        let p = pkgdir.as_path();

        self.write_kv(p, &mut file, "pkgname", &pkg.pkgname)?;
        self.write_kv(p, &mut file, "pkgbase", &pkgbuild.pkgbase)?;
        //self.write_kv(p, &mut file, "xdata", "pkgtype=pkg")?;
        self.write_kv(p, &mut file, "pkgver", &pkgbuild.version())?;

        self.write_kvs(p, &mut file, "pkgdesc", &pkg.pkgdesc)?;
        self.write_kvs(p, &mut file, "url", &pkg.url)?;
        self.write_kv(p, &mut file, "builddate", &c.source_date_epoch.to_string())?;
        self.write_kv(p, &mut file, "packager", &c.packager)?;
        self.write_kv(p, &mut file, "size", &size.to_string())?;
        self.write_kv(p, &mut file, "arch", &c.arch)?;

        self.write_kvs(p, &mut file, "license", &pkg.license)?;
        self.write_kvs(p, &mut file, "replaces", pkg.replaces.enabled(&c.arch))?;
        self.write_kvs(p, &mut file, "group", &pkg.groups)?;
        self.write_kvs(p, &mut file, "conflict", pkg.conflicts.enabled(&c.arch))?;
        self.write_kvs(p, &mut file, "provides", pkg.provides.enabled(&c.arch))?;
        self.write_kvs(p, &mut file, "backup", &pkg.backup)?;
        self.write_kvs(p, &mut file, "depend", pkg.depends.enabled(&c.arch))?;
        self.write_kvs(p, &mut file, "optdepend", pkg.optdepends.enabled(&c.arch))?;
        if !debug {
            self.write_kvs(
                p,
                &mut file,
                "makedepend",
                pkgbuild.makedepends.enabled(&c.arch),
            )?;
            self.write_kvs(
                p,
                &mut file,
                "checkdepend",
                pkgbuild.checkdepends.enabled(&c.arch),
            )?;
        }

        Ok(())
    }

    fn write_kvs<W, S, I>(&self, p: &Path, w: &mut W, key: &str, val: I) -> Result<()>
    where
        W: Write,
        S: AsRef<str>,
        I: IntoIterator<Item = S>,
    {
        for v in val {
            self.write_kv(p, w, key, v.as_ref())?;
        }

        Ok(())
    }

    fn write_kv<W: Write>(&self, p: &Path, w: &mut W, key: &str, val: &str) -> Result<()> {
        w.write_all(key.as_bytes())
            .and_then(|_| w.write_all(b" = "))
            .and_then(|_| w.write_all(val.as_bytes()))
            .and_then(|_| w.write_all(b"\n"))
            .context(Context::CreatePackage, IOContext::Write(p.to_path_buf()))?;
        Ok(())
    }

    fn package_size(&self, dirs: &PkgbuildDirs, pkg: &Package) -> Result<u64> {
        let path = dirs.pkgdir(pkg);
        let mut size = 0;
        let mut seen = HashSet::new();
        for file in walkdir::WalkDir::new(&path).follow_root_links(false) {
            let file = file.context(Context::GetPackageSize, IOContext::ReadDir(path.clone()))?;

            let metadata = file
                .metadata()
                .context(Context::GetPackageSize, IOContext::Stat(file.path().into()))?;

            if !file.file_type().is_file() {
                continue;
            }

            if seen.insert(metadata.ino()) {
                size += metadata.size();
            }
        }

        Ok(size)
    }

    pub fn package_files(&self, pkgdir: &Path) -> Result<Vec<u8>> {
        let mut files = Vec::new();
        let mut filesnull = Vec::new();

        for file in walkdir::WalkDir::new(pkgdir) {
            let file = file.context(Context::GetPackageFiles, IOContext::ReadDir(pkgdir.into()))?;

            let path = file.path().strip_prefix(pkgdir).unwrap();
            if path.is_empty() {
                continue;
            }

            files.push(path.to_path_buf());
        }

        files.sort_by(|a, b| a.as_os_str().cmp(b.as_os_str()));

        for path in files {
            filesnull.extend(path.as_os_str().as_bytes());
            filesnull.push(0);
        }

        Ok(filesnull)
    }

    fn copy_to_srcpkg(&self, from: &Path, to: &Path, name: &str) -> Result<()> {
        self.event(Event::AddingFileToPackage(name))?;
        copy_dir(from, to, Context::BuildPackage)?;
        Ok(())
    }

    pub fn create_source_package(
        &self,
        options: &Options,
        pkgbuild: &Pkgbuild,
        all: bool,
    ) -> Result<()> {
        let mut added = HashSet::new();
        umask(Mode::from_bits_truncate(0o022));

        self.event(Event::BuildingSourcePackage(
            &pkgbuild.pkgbase,
            &pkgbuild.version(),
        ))?;

        if !options.rebuild {
            self.err_if_srcpkg_built(options, pkgbuild)?;
        }

        let dirs = self.pkgbuild_dirs(pkgbuild)?;
        let start = dirs.startdir.as_path();
        let dest = dirs.srcpkgdir.as_path();

        self.download_sources(options, pkgbuild, true)?;
        self.check_integ(options, pkgbuild, true)?;

        self.event(Event::AddingPackageFiles)?;

        if dirs.srcpkgdir.exists() {
            rm_all(&dirs.srcpkgdir, Context::BuildPackage)?;
        }

        mkdir(&dirs.srcpkgdir, Context::BuildPackage)?;

        self.copy_to_srcpkg(&start.join("PKGBUILD"), &dest.join("PKGBUILD"), "PKGBUILD")?;
        self.event(Event::AddingFileToPackage(".SRCINFO"))?;
        write(
            dest.join(".SRCINFO"),
            pkgbuild.srcinfo(),
            Context::GenerateSrcinfo,
        )?;

        for pkg in pkgbuild.packages() {
            if let Some(i) = &pkg.install {
                if !added.insert(i) {
                    continue;
                }
                self.copy_to_srcpkg(&start.join(i), &dest.join(i), i)?;
            }

            if let Some(changelog) = &pkg.changelog {
                if !added.insert(changelog) {
                    continue;
                }
                self.copy_to_srcpkg(&start.join(changelog), &dest.join(changelog), changelog)?;
            }

            for fkey in &pkgbuild.validpgpkeys {
                let keyfile = format!("{}.asc", fkey);
                let key = Path::new("keys/pgp").join(&keyfile);
                if !dirs.startdir.join(&key).exists() {
                    self.log(LogLevel::Warning, LogMessage::KeyNotDoundInKeys(&keyfile))?;
                    continue;
                }

                let keydir = dest.join("keys/pgp");
                if !keydir.exists() {
                    mkdir(keydir, Context::BuildPackage)?;
                }

                self.copy_to_srcpkg(&start.join(&key), &dest.join(&key), &keyfile)?;
            }

            for arch in &pkgbuild.source.values {
                for sources in &arch.values {
                    if !sources.is_remote() || all {
                        self.copy_to_srcpkg(
                            &dirs.download_path(sources),
                            &dest.join(sources.file_name()),
                            sources.file_name(),
                        )?;
                    }
                }
            }

            if options.reproducible {
                for file in walkdir::WalkDir::new(dest) {
                    let file = file.context(
                        Context::CreatePackage,
                        IOContext::ReadDir(dest.to_path_buf()),
                    )?;
                    set_time(file.path(), self.config.source_date_epoch, false)?;
                }
            }

            self.make_archive(&dirs, pkgbuild, pkg, true)?;

            self.event(Event::BuiltSourcePackage(
                &pkgbuild.pkgbase,
                &pkgbuild.version(),
            ))?;
        }

        Ok(())
    }

    pub(crate) fn fakeroot_env(&self, command: &mut Command) -> Result<()> {
        let key = self.fakeroot()?;
        #[cfg(not(target_vendor = "apple"))]
        command.env("LD_LIBRARY_PATH", FAKEROOT_LIBDIRS);
        command.env("LD_PRELOAD", FakeRoot::library_name());
        #[cfg(target_vendor = "apple")]
        command
            .env("DYLD_FALLBACK_LIBRARY_PATH", FAKEROOT_LIBDIRS)
            .env("DYLD_INSERT_LIBRARIES", FakeRoot::library_name());
        command.env("FAKEROOTKEY", key);
        Ok(())
    }
}
