use std::{
    ffi::OsStr,
    fmt::Display,
    fs::read_dir,
    path::{Path, PathBuf},
    result::Result as StdResult,
    str::FromStr,
    time::{SystemTime, UNIX_EPOCH},
};

use nix::sys::stat::{umask, Mode};

pub use crate::lint_config::*;
use crate::{
    error::{Context, DownloadAgentError, LintError, LintKind, Result, VCSClientError},
    fs::{resolve_path, resolve_path_relative, Check},
    installation_variables::{MAKEPKG_CONFIG_PATH, PREFIX},
    pkgbuild::{OptionState, Options, Package, Pkgbuild, Source},
    raw::RawConfig,
    sources::VCSKind,
};

#[derive(Debug, Default, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Pkgext(pub Compress);

#[derive(Debug, Default, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Srcext(pub Compress);

#[derive(Debug, Default, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Compress {
    Cat,
    #[default]
    Gz,
    Bz2,
    Xz,
    Zst,
    Lzo,
    Lrz,
    Lz4,
    Z,
    Lz,
}

impl Display for Compress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.tarext())
    }
}

impl FromStr for Compress {
    type Err = LintKind;

    fn from_str(s: &str) -> StdResult<Self, Self::Err> {
        match s {
            ".tar" => Ok(Compress::Cat),
            ".tar.gz" => Ok(Compress::Gz),
            ".tar.b2" => Ok(Compress::Bz2),
            ".tar.xz" => Ok(Compress::Xz),
            ".tar.zst" => Ok(Compress::Zst),
            ".tar.lzo" => Ok(Compress::Lzo),
            ".tar.lrz" => Ok(Compress::Lrz),
            ".tar.lz4" => Ok(Compress::Lz4),
            ".tar.Z" => Ok(Compress::Z),
            ".tar.lz" => Ok(Compress::Lz),
            _ => Err(LintKind::InvalidPkgExt(s.to_string())),
        }
    }
}

impl Compress {
    pub fn tarext(&self) -> &'static str {
        match self {
            Compress::Cat => ".tar",
            Compress::Gz => ".tar.gz",
            Compress::Bz2 => ".tar.bz2",
            Compress::Xz => ".tar.xz",
            Compress::Zst => ".tar.zsr",
            Compress::Lzo => ".tar.lzo",
            Compress::Lrz => ".tar.lrz",
            Compress::Lz4 => ".tar.lz4",
            Compress::Z => ".tar.Z",
            Compress::Lz => ".tar.lz",
        }
    }
}

impl Display for Pkgext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(".pkg")?;
        self.0.fmt(f)
    }
}

impl FromStr for Pkgext {
    type Err = LintKind;

    fn from_str(s: &str) -> StdResult<Self, Self::Err> {
        let s = s
            .strip_prefix(".pkg")
            .ok_or_else(|| LintKind::InvalidPkgExt(s.to_string()))?;
        Ok(Self(s.parse()?))
    }
}

impl Pkgext {
    pub fn compress(&self) -> Compress {
        self.0
    }
}

impl Display for Srcext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(".src")?;
        self.0.fmt(f)
    }
}

impl FromStr for Srcext {
    type Err = LintKind;

    fn from_str(s: &str) -> StdResult<Self, Self::Err> {
        let s = s
            .strip_prefix(".src")
            .ok_or_else(|| LintKind::InvalidSrcExt(s.to_string()))?;
        Ok(Self(s.parse()?))
    }
}

impl Srcext {
    pub fn compress(&self) -> Compress {
        self.0
    }
}

#[derive(Debug, Clone, PartialOrd, Ord, PartialEq, Eq)]
pub struct VCSClient {
    pub protocol: VCSKind,
    pub package: String,
}

impl FromStr for VCSClient {
    type Err = VCSClientError;

    fn from_str(s: &str) -> StdResult<Self, Self::Err> {
        let (proto, package) = s.split_once("::").ok_or_else(|| VCSClientError {
            input: s.to_string(),
        })?;

        let protocol = proto.parse()?;

        let agent = Self {
            protocol,
            package: package.to_string(),
        };

        Ok(agent)
    }
}

#[derive(Debug, Clone, PartialOrd, Ord, PartialEq, Eq)]
pub struct DownloadAgent {
    pub protocol: String,
    pub command: String,
    pub args: Vec<String>,
}

impl FromStr for DownloadAgent {
    type Err = DownloadAgentError;

    fn from_str(s: &str) -> StdResult<Self, Self::Err> {
        let mut words = s.split_whitespace();
        let first = words.next().ok_or_else(|| DownloadAgentError {
            input: s.to_string(),
        })?;
        let (proto, command) = first.split_once("::").ok_or_else(|| DownloadAgentError {
            input: s.to_string(),
        })?;

        let agent = Self {
            protocol: proto.to_string(),
            command: command.to_string(),
            args: words.map(|s| s.to_string()).collect(),
        };

        Ok(agent)
    }
}

/// These are the paths that makepkg will use to run the build process and output package files.
///
/// By default makepkg will run the build and generate package files inside the PKGBUILD directory
/// unless explicitly configured in [`Config`] to use other directories.
///
/// This means each [`PkgbuildDirs`] is specific to the [`Config`] and [`Pkgbuild`] combination it was generated from.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PkgbuildDirs {
    /// The directory the [`Pkgbuild`] resides in.
    pub startdir: PathBuf,
    /// Full path to the [`Pkgbuild`] file.
    pub pkgbuild: PathBuf,
    /// Directory containing [`srcdir`](`PkgbuildDirs::srcdir`) and [`pkgdir`](`PkgbuildDirs::pkgdir`).
    /// If [`builddir`](`PkgbuildDirs::builddir`) is not set this will be the same as [`startdir`](`PkgbuildDirs::startdir`).
    pub builddir: PathBuf,
    /// The directory that sources are extracted to for the actual build to work with.
    /// This will be [`startdir`](`PkgbuildDirs::startdir`)/`src`, or if  [`builddir`](`PkgbuildDirs::builddir`) is set, [`builddir`](`PkgbuildDirs::builddir`)/[`pkgbase`](`Pkgbuild::pkgbase`)/`src`.
    pub srcdir: PathBuf,
    /// The directory that the build will places files into to be packages.
    /// Each package in the [`Pkgbuild`] writes to [`pkgdir`](`PkgbuildDirs::pkgdir`)/[`pkgname`](`Package::pkgname`).
    /// This will be [`startdir`](`PkgbuildDirs::startdir`)/`pkg`, or if [`builddir`](`PkgbuildDirs::builddir`) is set, [`builddir`](`PkgbuildDirs::builddir`)/[`pkgbase`](`Pkgbuild::pkgbase`)/`pkg`.
    pub pkgdir: PathBuf,
    /// The directory sources are downloaded to.
    pub srcdest: PathBuf,
    /// The directory the build package is created in.
    pub pkgdest: PathBuf,
    /// The directory built source packages are created in.
    pub srcpkgdest: PathBuf,
    /// The directory to write logfiles to. This is the same as [`startdir`](`PkgbuildDirs::startdir`) unless configured.
    pub logdest: PathBuf,
}

impl PkgbuildDirs {
    /// Gets the path that a [`Source`] would be downloaded to.
    ///
    /// This expands to [`srcdest`](`PkgbuildDirs::srcdest`)/[`filename`](`Source::file_name`) for remote
    /// sources and [`startdir`](`PkgbuildDirs::startdir`)/[`filename`](`Source::file_name`) for local sources.
    pub fn download_path(&self, source: &Source) -> PathBuf {
        if source.is_remote() {
            self.srcdest.join(source.file_name())
        } else {
            self.startdir.join(source.file_name())
        }
    }

    /// Gets the pkgdir for the specific [`Package`].
    ///
    /// This expands to [`pkgdir`](`PkgbuildDirs::pkgdir`)/[`pkgname`](`Package::pkgname`).
    pub fn pkgdir(&self, pkg: &Package) -> PathBuf {
        self.pkgdir.join(&pkg.pkgname)
    }
}

#[derive(Debug, Default)]
pub struct Config {
    pub dl_agents: Vec<DownloadAgent>,
    pub vcs_agents: Vec<VCSClient>,
    pub arch: String,
    pub chost: String,

    pub cppflags: String,
    pub cflags: String,
    pub cxxflags: String,
    pub rustflags: String,
    pub ldflags: String,
    pub ltoflags: String,
    pub makeflags: String,
    pub debug_cflags: String,
    pub debug_cxxflags: String,
    pub debug_rustflags: String,
    pub distcc_hosts: String,

    pub build_env: Options,
    pub options: Options,

    pub gpgkey: Option<String>,
    pub integrity_check: Vec<String>,
    pub strip_binaries: String,
    pub strip_shared: String,
    pub strip_static: String,
    pub man_dirs: Vec<PathBuf>,
    pub doc_dirs: Vec<PathBuf>,
    pub purge_targets: Vec<PathBuf>,
    pub dbg_srcdir: PathBuf,
    pub logdest: Option<PathBuf>,
    pub packager: String,
    pub compress_none: Vec<String>,
    pub compress_gz: Vec<String>,
    pub compress_bz2: Vec<String>,
    pub compress_xz: Vec<String>,
    pub compress_zst: Vec<String>,
    pub compress_lzo: Vec<String>,
    pub compress_lrz: Vec<String>,
    pub compress_lz4: Vec<String>,
    pub compress_z: Vec<String>,
    pub compress_lz: Vec<String>,
    pub pkgext: Pkgext,
    pub srcext: Srcext,
    pub pacman_auth: Vec<String>,

    pub builddir: Option<PathBuf>,
    pub srcdir: Option<PathBuf>,
    pub pkgdir: Option<PathBuf>,

    pub pkgdest: Option<PathBuf>,
    pub srcdest: Option<PathBuf>,
    pub srcpkgdest: Option<PathBuf>,

    pub source_date_epoch: u64,
    pub reproducable: bool,
    pub pacman: String,

    pub buildtool: String,
    pub buildtoolver: String,
}

impl Config {
    pub fn config_file() -> &'static Path {
        MAKEPKG_CONFIG_PATH.as_ref()
    }

    pub fn new() -> Result<Self> {
        Config::load(None)
    }

    pub fn with_path<P: Into<PathBuf>>(path: P) -> Result<Self> {
        Config::load(Some(path.into()))
    }

    pub fn compress_args(&self, compress: Compress) -> &[String] {
        match compress {
            Compress::Cat => self.compress_none.as_slice(),
            Compress::Gz => self.compress_gz.as_slice(),
            Compress::Bz2 => self.compress_bz2.as_slice(),
            Compress::Xz => self.compress_xz.as_slice(),
            Compress::Zst => self.compress_zst.as_slice(),
            Compress::Lzo => self.compress_lzo.as_slice(),
            Compress::Lrz => self.compress_lrz.as_slice(),
            Compress::Lz4 => self.compress_lz4.as_slice(),
            Compress::Lz => self.compress_lz.as_slice(),
            Compress::Z => self.compress_z.as_slice(),
        }
    }

    pub fn option(&self, pkgbuild: &Pkgbuild, name: &str) -> OptionState {
        match pkgbuild.options.get(name) {
            OptionState::Unset => self.options.get(name),
            state => state,
        }
    }

    pub fn build_option(&self, pkgbuild: &Pkgbuild, name: &str) -> OptionState {
        match pkgbuild.options.get(name) {
            OptionState::Unset => self.build_env.get(name),
            state => state,
        }
    }

    pub fn build_env(&self, name: &str) -> OptionState {
        self.build_env.get(name)
    }

    fn load(config: Option<PathBuf>) -> Result<Self> {
        umask(Mode::from_bits_truncate(0o022));

        let mut load_local = true;
        let mut conf_files = Vec::new();
        let mut lints = Vec::new();

        let main_config = if let Some(config) = config {
            load_local = false;
            config.to_path_buf()
        } else if let Ok(config) = std::env::var("MAKEPKG_CONF") {
            load_local = false;
            PathBuf::from(&config)
        } else {
            Self::config_file().to_path_buf()
        };

        Check::new(Context::ReadConfig).file().check(&main_config)?;

        let main_config = resolve_path(Context::ReadConfig, main_config)?;

        let mut configd = main_config.clone();
        configd.as_mut_os_string().push(".d");
        conf_files.push(main_config.to_path_buf().into_os_string());

        for file in read_dir(configd).into_iter().flatten().flatten() {
            if file.path().extension() == Some(OsStr::new(".conf"))
                && file.file_type().map(|t| !t.is_dir()).unwrap_or(false)
            {
                conf_files.push(file.file_name());
            }
        }

        if load_local {
            let path = dirs::config_dir()
                .map(|d| d.join("pacman/makepkg.conf"))
                .filter(|d| d.exists());

            if let Some(path) = path {
                conf_files.push(path.into_os_string());
            } else if let Some(home) = dirs::home_dir() {
                let path = home.join(".makepkg.conf");
                if path.exists() {
                    conf_files.push(path.into_os_string());
                }
            }
        }

        let source_date_epoch = match SystemTime::now().duration_since(UNIX_EPOCH) {
            Ok(epoch) => epoch.as_secs(),
            Err(e) => {
                lints.push(LintKind::InvalidSystemTime(e));
                1
            }
        };
        let packager = "Unknown packager".to_string();
        let pacman = "pacman".to_string();
        let buildtool = env!("CARGO_PKG_NAME").to_string();
        let buildtoolver = env!("CARGO_PKG_VERSION").to_string();
        let compress_none = to_string(&["cat"]);
        let compress_gz = to_string(&["gzip", "-c", "-f2", "-n"]);
        let compress_bz2 = to_string(&["bzip2", "-c", "-f"]);
        let compress_xz = to_string(&["xz", "-c", "-z", "-"]);
        let compress_zst = to_string(&["zstd", "-c", "-z", "-"]);
        let compress_lzo = to_string(&["lzop", "-q"]);
        let compress_lrz = to_string(&["lrzip", "-q"]);
        let compress_lz4 = to_string(&["lz4", "-q"]);
        let compress_z = to_string(&["compress", "-c", "-f"]);
        let compress_lz = to_string(&["lzip", "-c", "-f"]);
        let strip_shared = "-S".to_string();
        let strip_static = "-S".to_string();
        let ltoflags = "--flto".to_string();
        let dbg_srcdir = Path::new(PREFIX).join("src/debug");

        let mut config = Config {
            source_date_epoch,
            packager,
            pacman,
            buildtool,
            buildtoolver,
            dbg_srcdir,
            compress_none,
            compress_gz,
            compress_bz2,
            compress_xz,
            compress_zst,
            compress_lzo,
            compress_lrz,
            compress_lz4,
            compress_z,
            compress_lz,
            strip_shared,
            strip_static,
            ltoflags,
            ..Default::default()
        };

        let raw_config = RawConfig::from_paths(&conf_files)?;
        raw_config.lint(&mut lints);
        config.parse_raw(raw_config, &mut lints);

        if let Ok(pacman) = std::env::var("PACMAN") {
            config.pacman = pacman;
        }
        if let Ok(pkgdest) = std::env::var("PKGDEST") {
            config.pkgdest = Some(PathBuf::from(pkgdest));
        }
        if let Ok(srcdest) = std::env::var("SRCDEST") {
            config.srcdest = Some(PathBuf::from(srcdest));
        }
        if let Ok(srcpkgdest) = std::env::var("SRCPKGDEST") {
            config.srcpkgdest = Some(PathBuf::from(srcpkgdest));
        }
        if let Ok(logdest) = std::env::var("LOGDEST") {
            config.logdest = Some(logdest.into());
        }
        if let Ok(packager) = std::env::var("PACKAGER") {
            config.packager = packager;
        }
        if let Ok(builddir) = std::env::var("BUILDDIR") {
            config.builddir = Some(PathBuf::from(builddir));
        }
        if let Ok(carch) = std::env::var("CARCH") {
            config.arch = carch;
        }
        if let Ok(pkgext) = std::env::var("PKGEXT") {
            match pkgext.parse() {
                Ok(c) => config.pkgext = c,
                Err(e) => lints.push(e),
            }
        }
        if let Ok(srcext) = std::env::var("SRCEXT") {
            match srcext.parse() {
                Ok(c) => config.srcext = c,
                Err(e) => lints.push(e),
            }
        }
        if let Ok(key) = std::env::var("GPGKET") {
            config.gpgkey = Some(key);
        }
        if let Ok(epoch) = std::env::var("SOURCE_DATE_EPOCH") {
            config.source_date_epoch = epoch
                .parse()
                .map_err(|_| LintKind::InvalidEpoch(epoch).config())?;
            config.reproducable = true;
        }

        if let Ok(buildtool) = std::env::var("BUILDTOOL") {
            config.buildtool = buildtool;
        }
        if let Ok(buildtoolver) = std::env::var("BUILDTOOLVER") {
            config.buildtoolver = buildtoolver;
        }

        config.lint(&mut lints);

        if !lints.is_empty() {
            return Err(LintError::config(lints).into());
        }

        Ok(config)
    }

    pub fn pkgbuild_dirs(&self, pkgbuild: &Pkgbuild) -> Result<PkgbuildDirs> {
        let startdir = pkgbuild.dir.clone();

        let pkgbuild_file = startdir.join(Pkgbuild::file_name());
        let builddir = self
            .builddir
            .as_ref()
            .map(|dir| resolve_path_relative(dir, &startdir));

        let builddir = match builddir {
            Some(dir) if dir != startdir => dir.join(&pkgbuild.pkgbase),
            _ => startdir.clone(),
        };

        let logdest = match &self.logdest {
            Some(dir) => resolve_path_relative(dir, &startdir),
            _ => startdir.clone(),
        };

        let srcdir = builddir.join("src");
        let pkgdir = builddir.join("pkg");

        let pkgdest = self.pkgdest.as_ref().map_or_else(|| &startdir, |dir| dir);
        let srcdest = self.srcdest.as_ref().map_or_else(|| &startdir, |dir| dir);
        let srcpkgdest = self
            .srcpkgdest
            .as_ref()
            .map_or_else(|| &startdir, |dir| dir);

        let pkgdest = resolve_path_relative(pkgdest, &startdir);
        let srcdest = resolve_path_relative(srcdest, &startdir);
        let srcpkgdest = resolve_path_relative(srcpkgdest, &startdir);

        let dirs = PkgbuildDirs {
            startdir: startdir.to_path_buf(),
            pkgbuild: pkgbuild_file,
            builddir,
            srcdir,
            pkgdir,
            pkgdest,
            srcdest,
            srcpkgdest,
            logdest,
        };

        Ok(dirs)
    }

    fn parse_raw(&mut self, raw: RawConfig, lints: &mut Vec<LintKind>) {
        for var in raw.variables {
            match var.name.as_str() {
                "DLAGENTS" => {
                    self.dl_agents = var
                        .lint_array(lints)
                        .into_iter()
                        .filter_map(|s| match s.parse() {
                            Ok(v) => Some(v),
                            Err(e) => {
                                lints.push(LintKind::InvalidDownloadAgent(e));
                                None
                            }
                        })
                        .collect::<Vec<_>>();
                }
                "VCSCLIENTS" => {
                    self.vcs_agents = var
                        .lint_array(lints)
                        .into_iter()
                        .filter_map(|s| match s.parse() {
                            Ok(v) => Some(v),
                            Err(e) => {
                                lints.push(LintKind::InvalidVCSClient(e));
                                None
                            }
                        })
                        .collect::<Vec<_>>();
                }
                "CARCH" => self.arch = var.lint_string(lints),
                "CHOST" => self.chost = var.lint_string(lints),
                "CPPFLAGS" => self.cppflags = var.lint_string(lints),
                "CFLAGS" => self.cflags = var.lint_string(lints),
                "CXXFLAGS" => self.cxxflags = var.lint_string(lints),
                "RUSTFLAGS" => self.rustflags = var.lint_string(lints),
                "LDFLAGS" => self.ldflags = var.lint_string(lints),
                "LTOFLAGS" => self.ltoflags = var.lint_string(lints),
                "MAKEFLAGS" => self.makeflags = var.lint_string(lints),
                "DEBUG_CFLAGS" => self.debug_cflags = var.lint_string(lints),
                "DEBUG_CXXFLAGS" => self.debug_cxxflags = var.lint_string(lints),
                "DEBUG_RUSTFLAGS" => self.debug_rustflags = var.lint_string(lints),
                "BUILDENV" => {
                    self.build_env = var.lint_array(lints).iter().map(|s| s.as_str()).collect()
                }
                "DISTCC_HOSTS" => self.distcc_hosts = var.lint_string(lints),
                "BUILDDIR" => self.builddir = Some(PathBuf::from(var.lint_string(lints))),
                "GPGKEY" => self.gpgkey = Some(var.lint_string(lints)),
                "OPTIONS" => {
                    self.options = var.lint_array(lints).iter().map(|s| s.as_str()).collect()
                }
                "INTEGRITY_CHECK" => self.integrity_check = var.lint_array(lints),
                "STRIP_BINARIES" => self.strip_binaries = var.lint_string(lints),
                "STRIP_SHARED" => self.strip_shared = var.lint_string(lints),
                "STRIP_STATIC" => self.strip_static = var.lint_string(lints),
                "MAN_DIRS" => self.man_dirs = var.lint_path_array(lints),
                "DOC_DIRS" => self.doc_dirs = var.lint_path_array(lints),
                "PURGE_TARGETS" => self.purge_targets = var.lint_path_array(lints),
                "DBGSRCDIR" => self.dbg_srcdir = PathBuf::from(var.lint_string(lints)),
                "PKGDEST" => self.pkgdest = Some(PathBuf::from(var.lint_string(lints))),
                "SRCDEST" => self.srcdest = Some(PathBuf::from(var.lint_string(lints))),
                "SRCPKGDEST" => self.srcpkgdest = Some(PathBuf::from(var.lint_string(lints))),
                "LOGDEST" => self.logdest = Some(var.lint_string(lints).into()),
                "PACKAGER" => self.packager = var.lint_string(lints),
                "COMPRESSGZ" => self.compress_gz = var.lint_array(lints),
                "COMPRESSBZ2" => self.compress_bz2 = var.lint_array(lints),
                "COMPRESSXZ" => self.compress_xz = var.lint_array(lints),
                "COMPRESSZST" => self.compress_zst = var.lint_array(lints),
                "COMPRESSLZO" => self.compress_lzo = var.lint_array(lints),
                "COMPRESSLRZ" => self.compress_lrz = var.lint_array(lints),
                "COMPRESSZ" => self.compress_z = var.lint_array(lints),
                "COMPRESSLZ4" => self.compress_lz4 = var.lint_array(lints),
                "COMPRESSLZ" => self.compress_lz = var.lint_array(lints),
                "PKGEXT" => match var.lint_string(lints).parse() {
                    Ok(ext) => self.pkgext = ext,
                    Err(e) => lints.push(e),
                },
                "SRCEXT" => match var.lint_string(lints).parse() {
                    Ok(ext) => self.srcext = ext,
                    Err(e) => lints.push(e),
                },
                "PACMAN_AUTH" => self.pacman_auth = var.lint_array(lints),
                _ => (),
            }
        }
    }
}

fn to_string(s: &[&str]) -> Vec<String> {
    s.iter().map(|s| s.to_string()).collect()
}
