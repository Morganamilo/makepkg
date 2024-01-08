use std::{
    ffi::OsStr,
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
    pkgbuild::{Options, Pkgbuild, Source},
    raw::RawConfig,
};

#[derive(Debug, Default, Clone, PartialOrd, Ord, PartialEq, Eq)]
pub struct VCSClient {
    pub protocol: String,
    pub package: String,
}

impl FromStr for VCSClient {
    type Err = VCSClientError;

    fn from_str(s: &str) -> StdResult<Self, Self::Err> {
        let (proto, package) = s.split_once("::").ok_or_else(|| VCSClientError {
            input: s.to_string(),
        })?;

        let agent = Self {
            protocol: proto.to_string(),
            package: package.to_string(),
        };

        Ok(agent)
    }
}

#[derive(Debug, Default, Clone, PartialOrd, Ord, PartialEq, Eq)]
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

#[derive(Debug, Default)]
pub struct PkgbuildDirs {
    /// The directory the `PKGBUILD` resides in.
    pub startdir: PathBuf,
    /// Full path to the `PKGBUILD` file.
    pub pkgbuild: PathBuf,
    /// Directory containing `SRCDIR` and `PKGDIR`.
    /// If `BUILDDIR` is not set this will be the same as `STARTDIR`.
    pub builddir: PathBuf,
    /// The directory that sources are extracted to for the actual build to work with.
    /// This will be `STARTDIR/src`, or if `BUILDDIR` is set, `BUILDDIR/PKGBASE/src`.
    pub srcdir: PathBuf,
    /// The directory that the build will places files into to be packages.
    /// Each package in the `PKGBUILD` writes to `PKGDIR/PKGNAME`.
    /// This will be `STARTDIR/pkg`, or if `BUILDDIR` is set, `BUILDDIR/PKGBASE/pkg`.
    pub pkgdir: PathBuf,
    /// The directory sources are downloaded to.
    pub srcdest: PathBuf,
    /// The directory the build package is created in.
    pub pkgdest: PathBuf,
    /// The directory built source packages are created in.
    pub srcpkgdest: PathBuf,
}

impl PkgbuildDirs {
    pub fn download_path(&self, source: &Source) -> PathBuf {
        if source.is_download() {
            self.srcdest.join(source.file_name())
        } else {
            self.startdir.join(source.file_name())
        }
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
    pub logdest: PathBuf,
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
    pub pkgext: String,
    pub srcext: String,
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
    pub fn new() -> Result<Self> {
        Config::load(None)
    }

    pub fn with_path<P: Into<PathBuf>>(path: P) -> Result<Self> {
        Config::load(Some(path.into()))
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
            Path::new("/etc/makepkg.conf").to_path_buf()
        };

        Check::new(Context::ReadConfig).read().check(&main_config)?;

        let main_config = resolve_path(main_config)?;

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
        let dbg_srcdir = PathBuf::from("/usr/src/debug");
        let compress_none = vec!["cat".to_string()];
        let strip_shared = "-S".to_string();
        let strip_static = "-S".to_string();

        let mut config = Config {
            source_date_epoch,
            packager,
            pacman,
            buildtool,
            buildtoolver,
            dbg_srcdir,
            compress_none,
            strip_shared,
            strip_static,
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
            config.logdest = logdest.into();
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
            config.pkgext = pkgext;
        }
        if let Ok(srcext) = std::env::var("SRCEXT") {
            config.srcext = srcext;
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
                "LOGDEST" => self.logdest = var.lint_string(lints).into(),
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
                "PKGEXT" => self.pkgext = var.lint_string(lints),
                "SRCEXT" => self.srcext = var.lint_string(lints),
                "PACMAN_AUTH" => self.pacman_auth = var.lint_array(lints),
                _ => (),
            }
        }
    }
}
