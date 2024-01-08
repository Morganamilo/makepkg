use nix::NixPath;
use sha2::Sha256;

use crate::{
    callback::Event,
    config::PkgbuildDirs,
    error::{CommandErrorExt, Context, IOContext, IOErrorExt, LintError, LintKind, Result},
    fs::{copy, open, set_time},
    installation_variables::FAKEROOT_LIBDIRS,
    integ::hash_file,
    options::Options,
    pacman::buildinfo_installed,
    pkgbuild::{Package, Pkgbuild},
    FakeRoot, Makepkg,
};

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
    thread,
};

#[derive(Debug, PartialEq, Eq)]
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
            self.event(Event::CreatingDebugPackage(pkg.pkgname.to_string()));
        } else {
            self.event(Event::CreatingPackage(pkg.pkgname.to_string()));
        }

        let pkgdir = dirs.pkgdir(pkg);

        self.generate_pkginfo(dirs, pkgbuild, pkg, debug)?;
        self.generate_buildinfo(dirs, pkgbuild, pkg)?;

        if let Some(install) = &pkg.install {
            let dest = pkgdir.join(".INSTALL");
            self.event(Event::AddingFileTopackage(install.to_string()));
            let install = dirs.startdir.join(install);
            copy(install, &dest, Context::CreatePackage)?;
            std::fs::set_permissions(&dest, PermissionsExt::from_mode(0o644))
                .context(Context::CreatePackage, IOContext::Chmod(dest))?;
        }

        if let Some(changelog) = &pkg.changelog {
            self.event(Event::AddingFileTopackage(changelog.to_string()));
            let changelog = dirs.startdir.join(changelog);
            let dest = pkgdir.join(".CHANGELOG");
            copy(changelog, &dest, Context::CreatePackage)?;
            std::fs::set_permissions(&dest, PermissionsExt::from_mode(0o644))
                .context(Context::CreatePackage, IOContext::Chmod(dest))?;
        }

        for file in walkdir::WalkDir::new(&pkgdir) {
            let file = file.context(Context::CreatePackage, IOContext::ReadDir(pkgdir.clone()))?;
            set_time(file.path(), self.config.source_date_epoch)?;
        }

        self.generate_mtree(dirs, pkg)?;

        set_time(pkgdir.join(".MTREE"), self.config.source_date_epoch)?;

        if !options.no_archive {
            self.make_archive(dirs, pkgbuild, pkg, false)?;
        }

        Ok(())
    }

    fn generate_mtree(&self, dirs: &PkgbuildDirs, pkg: &Package) -> Result<()> {
        self.event(Event::GeneratingPackageFile(".MTREE".to_string()));
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

        let mut tar = tarcmd
            .spawn()
            .cmd_context(&tarcmd, Context::GeneratePackageFile(".MTREE".into()))?;
        let mut tar_in = tar.stdin.take().unwrap();

        let thread = thread::spawn(move || tar_in.write_all(&files));

        let mut gzip = Command::new("gzip");
        gzip.arg("-cfn")
            .stdout(mtree)
            .stdin(tar.stdout.take().unwrap());

        let mut gzip = gzip
            .spawn()
            .cmd_context(&tarcmd, Context::GeneratePackageFile(".MTREE".into()))?;

        tar.wait()
            .cmd_context(&tarcmd, Context::GeneratePackageFile(".MTREE".into()))?;

        gzip.wait()
            .cmd_context(&tarcmd, Context::GeneratePackageFile(".MTREE".into()))?;

        thread.join().unwrap().context(
            Context::CreatePackage,
            IOContext::WriteProcess("bsdtar".to_string()),
        )?;

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
        let pkgext;
        let pkgname;
        let pkgfilename;
        let pkgfile;

        if srcpkg {
            pkgname = pkgbuild.pkgbase.as_str();
            pkgdir = dirs.startdir.join("srcpkg");
            pkgext = self.config.srcext.as_str();
            pkgfilename = format!("{}-{}{}", pkgname, pkgbuild.version(), pkgext);
            pkgfile = dirs.srcpkgdest.join(&pkgfilename);
        } else {
            pkgname = pkg.pkgname.as_str();
            pkgdir = dirs.pkgdir(pkg);
            pkgext = self.config.pkgext.as_str();
            pkgfilename = format!(
                "{}-{}-{}{}",
                pkgname,
                pkgbuild.version(),
                self.config.arch,
                pkgext
            );
            pkgfile = dirs.srcpkgdest.join(&pkgfilename)
        };

        let compress = self.compress()?;
        let compress_prog = compress.get(0).ok_or_else(|| {
            LintError::config(vec![LintKind::VariabeContainsEmptyString(
                "COMPRESS".to_string(),
            )])
        })?;

        self.event(Event::GeneratingPackageFile(pkgfilename.clone()));
        let files = self.package_files(&pkgdir)?;

        let mut file = File::options();
        file.create(true).write(true).truncate(true);
        let pkgfile = open(&file, pkgfile, Context::CreatePackage)?;

        let mut tarcmd = Command::new("bsdtar");
        self.fakeroot_env(&mut tarcmd)?;
        tarcmd
            .arg("--no-fflags")
            .arg("-cnf")
            .arg("-")
            .arg("--null")
            .arg("--files-from")
            .arg("-")
            .env("LANG", "C")
            .current_dir(&pkgdir)
            .stdout(Stdio::piped())
            .stdin(Stdio::piped());

        let mut tar = tarcmd
            .spawn()
            .cmd_context(&tarcmd, Context::CreatePackage)?;

        let mut tar_in = tar.stdin.take().unwrap();

        let thread = thread::spawn(move || tar_in.write_all(&files));

        let mut zipcmd = Command::new(compress_prog);
        zipcmd
            .args(&compress[1..])
            .stdout(pkgfile)
            .stdin(tar.stdout.take().unwrap());

        let mut zip = zipcmd
            .spawn()
            .cmd_context(&zipcmd, Context::CreatePackage)?;

        tar.wait().cmd_context(&tarcmd, Context::CreatePackage)?;
        zip.wait().cmd_context(&zipcmd, Context::CreatePackage)?;
        thread.join().unwrap().context(
            Context::CreatePackage,
            IOContext::WriteProcess("bsdtar".into()),
        )?;

        Ok(())
    }

    fn generate_buildinfo(
        &self,
        dirs: &PkgbuildDirs,
        pkgbuild: &Pkgbuild,
        pkg: &Package,
    ) -> Result<()> {
        self.event(Event::GeneratingPackageFile(".BUILDINFO".to_string()));
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

        let installed = buildinfo_installed();

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
        self.event(Event::GeneratingPackageFile(".PKGINFO".to_string()));

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
        let fakeroot = fakerootcmd.arg("-v").output().cmd_context(
            &fakerootcmd,
            Context::GeneratePackageFile(".PKGINFO".into()),
        )?;

        let fakeroot = String::from_utf8(fakeroot.stdout).cmd_context(
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

    fn compress(&self) -> Result<&[String]> {
        let c = self.config();

        let flags = match c.pkgext.as_str() {
            ".pkg.tar" => c.compress_none.as_slice(),
            ".pkg.tar.gz" => c.compress_gz.as_slice(),
            ".pkg.tar.b2" => c.compress_bz2.as_slice(),
            ".pkg.tar.xz" => c.compress_xz.as_slice(),
            ".pkg.tar.zst" => c.compress_zst.as_slice(),
            ".pkg.tar.lzo" => c.compress_lzo.as_slice(),
            ".pkg.tar.lrz" => c.compress_lrz.as_slice(),
            ".pkg.tar.lz4" => c.compress_lz4.as_slice(),
            ".pkg.tar.lz" => c.compress_lz.as_slice(),
            ".pkg.tar.Z" => c.compress_z.as_slice(),
            ext => {
                return Err(
                    LintError::config(vec![LintKind::InvalidPkgExt(ext.to_string())]).into(),
                )
            }
        };

        Ok(flags)
    }

    pub fn create_source_package(
        &self,
        options: &Options,
        pkgbuild: &Pkgbuild,
        all: bool,
    ) -> Result<()> {
        self.err_if_srcpkg_built(options, pkgbuild)?;

        self.event(Event::BuildingSourcePackage(
            pkgbuild.pkgbase.to_string(),
            pkgbuild.version(),
        ));

        self.download_sources(options, pkgbuild, all)?;
        self.check_integ(options, pkgbuild, all)?;

        self.event(Event::BuiltSourcePackage(
            pkgbuild.pkgbase.clone(),
            pkgbuild.version(),
        ));

        Ok(())
    }

    pub(crate) fn fakeroot_env(&self, command: &mut Command) -> Result<()> {
        let key = self.fakeroot()?;
        #[cfg(not(os_family = "apple"))]
        command.env("LD_LIBRARY_PATH", FAKEROOT_LIBDIRS);
        command.env("LD_PRELOAD", FakeRoot::library_name());
        #[cfg(os_family = "apple")]
        command
            .env("DYLD_FALLBACK_LIBRARY_PATH", FAKEROOT_LIBDIRS)
            .env("DYLD_INSERT_LIBRARIES", FakeRoot::library_name());
        command.env("FAKEROOTKEY", key);
        Ok(())
    }
}
