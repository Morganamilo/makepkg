use std::{
    collections::HashSet,
    fmt::Display,
    fs::read_to_string,
    path::{Path, PathBuf},
    result::Result as StdResult,
    str::FromStr,
};

use crate::{
    config::Config,
    error::{Context, Error, IOContext, IOErrorExt, LintError, LintKind, Result},
    fs::{resolve_path, Check},
    lint_pkgbuild::check_pkgver,
    raw::{FunctionVariables, RawPkgbuild, Value, Variable},
};

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Function {
    Verify,
    Prepare,
    Pkgver,
    Build,
    Check,
    Package,
}

impl Display for Function {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

impl Function {
    fn new(s: &str) -> Option<Self> {
        match s {
            "verify" => Some(Function::Verify),
            "prepare" => Some(Function::Prepare),
            "pkgver" => Some(Function::Pkgver),
            "build" => Some(Function::Build),
            "check" => Some(Function::Check),
            "package" => Some(Function::Package),
            name if name.starts_with("package_") => Some(Function::Package),
            _ => None,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Function::Verify => "verify",
            Function::Prepare => "prepare",
            Function::Pkgver => "pkgver",
            Function::Build => "build",
            Function::Check => "check",
            Function::Package => "package",
        }
    }
}

#[derive(Debug, Default, Clone, Eq, PartialEq, Hash)]
struct Key {
    name: String,
    arch: Option<String>,
}

#[derive(Debug, Default, Clone)]
pub struct ArchVecs<T> {
    pub values: Vec<ArchVec<T>>,
}

impl<T> ArchVecs<T> {
    pub fn all(&self) -> impl Iterator<Item = &T> {
        self.values.iter().flat_map(|v| &v.values)
    }

    pub fn enabled<'a>(&'a self, arch: &'a str) -> impl Iterator<Item = &'a T> {
        self.values
            .iter()
            .filter(|v| v.enabled(arch))
            .take(2)
            .flat_map(|v| &v.values)
    }

    pub fn get(&self, arch: Option<&str>) -> Option<&ArchVec<T>> {
        self.values.iter().find(|v| v.arch.as_deref() == arch)
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    pub fn push(&mut self, value: ArchVec<T>) {
        self.values.push(value)
    }

    pub fn clear(&mut self) {
        self.values.clear();
    }
}

impl ArchVecs<String> {
    pub fn merge(&mut self, other: Variable) -> StdResult<(), LintKind> {
        let other = other.get_arch_array()?;
        if let Some(oldval) = self.values.iter_mut().find(|v| v.arch == other.arch) {
            *oldval = other;
        } else {
            self.values.push(other);
        }

        Ok(())
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ArchVec<T> {
    pub arch: Option<String>,
    pub values: Vec<T>,
}

impl<T> ArchVec<T> {
    pub fn enabled(&self, arch: &str) -> bool {
        match &self.arch {
            Some(a) => a == arch,
            None => true,
        }
    }

    pub fn from_vec<S: Into<String>>(arch: Option<S>, vec: Vec<T>) -> Self {
        Self {
            arch: arch.map(|s| s.into()),
            values: vec,
        }
    }
}

#[derive(Debug, Default, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum OptionState {
    Enabled,
    Disabled,
    #[default]
    Unset,
}

impl OptionState {
    pub fn enabled(self) -> bool {
        self == OptionState::Enabled
    }

    pub fn disabled(self) -> bool {
        self == OptionState::Disabled
    }

    pub fn unset(self) -> bool {
        self == OptionState::Unset
    }
}

#[derive(Debug, Default, Clone)]
pub struct Options {
    pub values: Vec<OptionValue>,
}

impl<'a> FromIterator<&'a str> for Options {
    fn from_iter<T: IntoIterator<Item = &'a str>>(iter: T) -> Self {
        let values = iter.into_iter().map(OptionValue::new).collect();
        Options { values }
    }
}

impl Options {
    pub fn get(&self, name: &str) -> OptionState {
        match self.values.iter().find(|o| o.name == name) {
            Some(v) if v.enabled => OptionState::Enabled,
            Some(_) => OptionState::Disabled,
            None => OptionState::Unset,
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OptionValue {
    pub name: String,
    pub enabled: bool,
}

impl Display for OptionValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if !self.enabled {
            f.write_str("!")?;
        }

        f.write_str(&self.name)
    }
}

impl OptionValue {
    pub fn new(name: &str) -> Self {
        if let Some(name) = name.strip_prefix('!') {
            OptionValue {
                name: name.to_string(),
                enabled: false,
            }
        } else {
            OptionValue {
                name: name.to_string(),
                enabled: true,
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Fragment {
    Revision(String),
    Branch(String),
    Commit(String),
    Tag(String),
}

impl Display for Fragment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}={}", self.key(), self.value())
    }
}

impl FromStr for Fragment {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        let frag = match s.split_once('=') {
            Some(("revision", v)) => Fragment::Revision(v.to_string()),
            Some(("branch", v)) => Fragment::Branch(v.to_string()),
            Some(("commit", v)) => Fragment::Commit(v.to_string()),
            Some(("tag", v)) => Fragment::Tag(v.to_string()),
            _ => return Err(LintKind::UnknownFragment(s.to_string()).pkgbuild().into()),
        };

        Ok(frag)
    }
}

impl Fragment {
    pub fn key(&self) -> &'static str {
        match self {
            Fragment::Revision(_) => "revision",
            Fragment::Branch(_) => "branch",
            Fragment::Commit(_) => "commit",
            Fragment::Tag(_) => "tag",
        }
    }

    pub fn value(&self) -> &str {
        match self {
            Fragment::Revision(s)
            | Fragment::Branch(s)
            | Fragment::Commit(s)
            | Fragment::Tag(s) => s.as_str(),
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Source {
    pub filename_override: Option<String>,
    pub proto_prefix: Option<String>,
    pub url: String,
    pub fragment: Option<Fragment>,
    pub query: Option<String>,
}

impl Display for Source {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(filename) = &self.filename_override {
            f.write_str(filename)?;
            f.write_str("::")?;
        }
        if let Some(proto) = &self.proto_prefix {
            f.write_str(proto)?;
            f.write_str("+")?;
        }
        f.write_str(&self.url)?;
        if let Some(fragment) = &self.fragment {
            f.write_str("#")?;
            f.write_str(&fragment.to_string())?;
        }
        if let Some(query) = &self.query {
            f.write_str("?")?;
            f.write_str(query)?;
        }
        Ok(())
    }
}

// TODO: do this proper
impl Source {
    pub fn new(url: &str) -> Self {
        let (filename, url) = match url.split_once("::") {
            Some((filename, url)) => (Some(filename), url),
            None => (None, url),
        };

        if let Some((proto, _)) = url.split_once("://") {
            let (proto_prefix, proto) = match proto.split_once('+') {
                Some((proto_prefix, proto)) => (Some(proto_prefix.to_owned()), proto),
                None => (None, proto),
            };

            let url = url.split_once('+').map(|s| s.1).unwrap_or(url);

            let main_proto = proto_prefix.as_deref().unwrap_or(proto);

            if ["git", "bzr", "svn", "hg", "fossil"].contains(&main_proto) {
                let (url, query) = match url.split_once('?') {
                    Some((url, query)) => (url, Some(query)),
                    None => (url, None),
                };

                let (url, fragment) = match url.split_once('#') {
                    // TODO error on invalid fragment
                    Some((url, fragment)) => (url, fragment.parse().ok()),
                    None => (url, None),
                };
                return Source {
                    filename_override: filename.map(|s| s.to_string()),
                    url: url.to_string(),
                    fragment,
                    query: query.map(|s| s.to_string()),
                    proto_prefix,
                };
            }
        }

        Source {
            filename_override: filename.map(|s| s.to_string()),
            url: url.to_string(),
            fragment: None,
            query: None,
            proto_prefix: None,
        }
    }

    pub fn protocol(&self) -> Option<&str> {
        self.proto_prefix
            .as_deref()
            .or_else(|| self.url.split_once("://").map(|u| u.0))
    }

    pub fn is_remote(&self) -> bool {
        self.url.contains("://")
    }

    pub fn file_name(&self) -> &str {
        let mut filename = if let Some(filename) = &self.filename_override {
            filename.as_str()
        } else {
            self.url.rsplit('/').next().unwrap()
        };

        if self.protocol() == Some("git") {
            filename = filename.trim_end_matches(".git");
        }
        filename
    }
}

#[derive(Debug, Default, Clone)]
pub struct Pkgbuild {
    pub pkgbase: String,
    pub pkgver: String,
    pub pkgrel: String,
    pub epoch: Option<String>,
    pub pkgdesc: Option<String>,
    pub url: Option<String>,
    pub license: Vec<String>,
    pub install: Option<String>,
    pub changelog: Option<String>,
    pub source: ArchVecs<Source>,
    pub validpgpkeys: Vec<String>,
    pub noextract: Vec<String>,
    pub md5sums: ArchVecs<String>,
    pub sha1sums: ArchVecs<String>,
    pub sha224sums: ArchVecs<String>,
    pub sha256sums: ArchVecs<String>,
    pub sha384sums: ArchVecs<String>,
    pub sha512sums: ArchVecs<String>,
    pub b2sums: ArchVecs<String>,
    pub groups: Vec<String>,
    pub arch: Vec<String>,
    pub backup: Vec<String>,
    pub depends: ArchVecs<String>,
    pub makedepends: ArchVecs<String>,
    pub checkdepends: ArchVecs<String>,
    pub optdepends: ArchVecs<String>,
    pub conflicts: ArchVecs<String>,
    pub provides: ArchVecs<String>,
    pub replaces: ArchVecs<String>,
    pub options: Options,
    pub packages: Vec<Package>,
    pub functions: Vec<Function>,
    pub dir: PathBuf,
    pub(crate) package_functions: Vec<String>,
}

#[derive(Debug, Default, Clone)]
pub struct Package {
    pub pkgname: String,
    pub pkgdesc: Option<String>,
    pub url: Option<String>,
    pub license: Vec<String>,
    pub install: Option<String>,
    pub changelog: Option<String>,
    pub groups: Vec<String>,
    pub arch: Vec<String>,
    pub backup: Vec<String>,
    pub depends: ArchVecs<String>,
    pub optdepends: ArchVecs<String>,
    pub conflicts: ArchVecs<String>,
    pub provides: ArchVecs<String>,
    pub replaces: ArchVecs<String>,
    pub options: Options,
    overridden: HashSet<Key>,
}

impl Pkgbuild {
    pub fn file_name() -> &'static str {
        "PKGBUILD"
    }

    pub fn has_function(&self, func: Function) -> bool {
        self.functions.iter().any(|f| *f == func)
    }

    pub fn version(&self) -> String {
        if let Some(epoch) = &self.epoch {
            format!("{}:{}-{}", epoch, self.pkgver, self.pkgrel)
        } else {
            format!("{}-{}", self.pkgver, self.pkgrel)
        }
    }

    pub fn packages(&self) -> impl Iterator<Item = &Package> {
        self.packages.iter()
    }

    pub fn pkgnames(&self) -> impl Iterator<Item = &str> {
        self.packages.iter().map(|p| p.pkgname.as_str())
    }

    pub fn set_pkgver<S: Into<String>>(&mut self, path: &Path, pkgver: S) -> Result<()> {
        let mut lints = Vec::new();
        let pkgver = pkgver.into();
        check_pkgver(&pkgver, "pkgver", &mut lints);

        if !lints.is_empty() {
            return Err(LintError::pkgbuild(lints).into());
        }

        if pkgver != self.pkgver && self.pkgrel != "1" {
            Pkgbuild::set_var(path, "pkgrel", "1")?;
        }

        self.pkgver = pkgver;
        Pkgbuild::set_var(path, "pkgver", &self.pkgver)?;
        Ok(())
    }

    fn set_var(path: &Path, name: &str, val: &str) -> Result<()> {
        let contents = read_to_string(path).context(
            Context::SetPkgbuildVar("pkgver".to_string()),
            IOContext::Read(path.to_path_buf()),
        )?;
        let mut edited = String::new();
        let name = format!("{}=", name);

        for line in contents.lines() {
            if line.starts_with(&name) {
                let split = line.split_once(char::is_whitespace);
                edited.push_str("pkgver=");
                edited.push_str(val);
                if let Some((_, rest)) = split {
                    edited.push(' ');
                    edited.push_str(rest);
                }
            } else {
                edited.push_str(line);
            }
            edited.push('\n');
        }

        std::fs::write(path, edited).context(
            Context::SetPkgbuildVar("pkgver".to_string()),
            IOContext::Write(path.to_path_buf()),
        )?;

        Ok(())
    }

    pub fn new<P: Into<PathBuf>>(dir: P) -> Result<Self> {
        let dir = dir.into();
        let dir = resolve_path(Context::ReadPkgbuild, dir)?;
        let pkgbuild_path = dir.join(Pkgbuild::file_name());

        Check::new(Context::ReadPkgbuild).dir().check(&dir)?;
        Check::new(Context::ReadPkgbuild)
            .file()
            .check(&pkgbuild_path)?;

        let raw = RawPkgbuild::from_path(pkgbuild_path)?;
        let mut pkgbuild = Pkgbuild::default();
        let mut packages = Vec::new();
        let mut lints = Vec::new();
        pkgbuild.dir = dir;

        raw.lint(&mut lints);

        for var in raw.variables {
            pkgbuild.process_global_var(var, &mut packages, &mut lints);
        }

        for pkgname in packages {
            pkgbuild.add_package(pkgname);
        }

        for func in raw.function_variables {
            pkgbuild.process_function_vars(func, &mut lints);
        }

        if pkgbuild.pkgbase.is_empty() {
            pkgbuild.pkgbase = pkgbuild.packages[0].pkgname.clone();
        }

        pkgbuild.functions = raw
            .functions
            .iter()
            .filter_map(|f| Function::new(f))
            .collect();

        pkgbuild.package_functions = raw
            .functions
            .into_iter()
            .filter(|f| f.starts_with("package"))
            .collect();

        pkgbuild.functions.sort();
        pkgbuild.functions.dedup();

        pkgbuild.lint(&mut lints);

        if !lints.is_empty() {
            return Err(LintError::pkgbuild(lints).into());
        }

        Ok(pkgbuild)
    }

    fn process_global_var(
        &mut self,
        var: Variable,
        packages: &mut Vec<String>,
        lints: &mut Vec<LintKind>,
    ) {
        let name = var.name.clone();

        match name.as_str() {
            "pkgname" => {
                var.lint_no_arch(lints);
                let names = match var.value {
                    Value::String(s) => vec![s],
                    Value::Array(a) => a,
                    Value::Map(_) => {
                        lints.push(LintKind::WrongValueType(
                            var.name_arch(),
                            "string or array".to_string(),
                            "map".to_string(),
                        ));
                        Vec::new()
                    }
                };

                *packages = names;
            }
            "pkgver" => self.pkgver = var.lint_string(lints),
            "pkgrel" => self.pkgrel = var.lint_string(lints),
            "epoch" => self.epoch = Some(var.lint_string(lints)),
            "pkgdesc" => self.pkgdesc = Some(var.lint_string(lints)),
            "url" => self.url = Some(var.lint_string(lints)),
            "license" => self.license = var.lint_array(lints),
            "install" => self.install = Some(var.lint_string(lints)),
            "changelog" => self.changelog = Some(var.lint_string(lints)),
            "source" => {
                let array = var.lint_arch_array(lints);
                let arch = array.arch;
                let array = array
                    .values
                    .into_iter()
                    .map(|url| Source::new(&url))
                    .collect();
                let array = ArchVec {
                    arch,
                    values: array,
                };
                self.source.push(array);
            }
            "validpgpkeys" => self.validpgpkeys = var.lint_array(lints),
            "noextract" => self.noextract = var.lint_array(lints),
            "md5sums" => self.md5sums.push(var.lint_arch_array(lints)),
            "sha1sums" => self.sha1sums.push(var.lint_arch_array(lints)),
            "sha224sums" => self.sha224sums.push(var.lint_arch_array(lints)),
            "sha256sums" => self.sha256sums.push(var.lint_arch_array(lints)),
            "sha384sums" => self.sha384sums.push(var.lint_arch_array(lints)),
            "sha512sums" => self.sha512sums.push(var.lint_arch_array(lints)),
            "b2sums" => self.b2sums.push(var.lint_arch_array(lints)),
            "groups" => self.groups = var.lint_array(lints),
            "arch" => self.arch = var.lint_array(lints),
            "backup" => self.backup = var.lint_array(lints),
            "depends" => self.depends.push(var.lint_arch_array(lints)),
            "makedepends" => self.makedepends.push(var.lint_arch_array(lints)),
            "checkdepends" => self.checkdepends.push(var.lint_arch_array(lints)),
            "optdepends" => self.optdepends.push(var.lint_arch_array(lints)),
            "conflicts" => self.conflicts.push(var.lint_arch_array(lints)),
            "provides" => self.provides.push(var.lint_arch_array(lints)),
            "replaces" => self.replaces.values.push(var.lint_arch_array(lints)),
            "options" => self.options = var.lint_array(lints).iter().map(|s| s.as_str()).collect(),
            _ => (),
        }
    }

    fn process_function_vars(&mut self, func: FunctionVariables, lints: &mut Vec<LintKind>) {
        let package_name = if func.function_name == "package" {
            self.packages[0].pkgname.clone()
        } else {
            func.function_name
                .trim_start_matches("package_")
                .to_string()
        };

        let Some(package) = self.packages.iter_mut().find(|p| p.pkgname == package_name) else {
            return;
        };

        for var in func.variables {
            let name = var.name.to_string();
            let name = name.as_str();

            set_override_flag(package, &var);

            match name {
                "pkgdesc" => package.pkgdesc = Some(var.lint_string(lints)),
                "arch" => package.arch = var.lint_array(lints),
                "url" => package.url = Some(var.lint_string(lints)),
                "license" => package.license = var.lint_array(lints),
                "groups" => package.groups = var.lint_array(lints),
                "depends" => package.depends.lint_merge(var, lints),
                "optdepends" => package.optdepends.lint_merge(var, lints),
                "provides" => package.provides.lint_merge(var, lints),
                "conflicts" => package.conflicts.lint_merge(var, lints),
                "replaces" => package.replaces.lint_merge(var, lints),
                "backup" => package.backup = var.lint_array(lints),
                "install" => package.install = Some(var.lint_string(lints)),
                "changelog" => package.changelog = Some(var.lint_string(lints)),
                "options" => {
                    self.options = var.lint_array(lints).iter().map(|s| s.as_str()).collect()
                }

                _ => (),
            }
        }
    }

    pub fn add_package(&mut self, pkgname: String) -> &mut Package {
        self.packages.push(self.new_package(pkgname));
        self.packages.last_mut().unwrap()
    }

    pub fn new_package(&self, pkgname: String) -> Package {
        Package {
            pkgname,
            pkgdesc: self.pkgdesc.clone(),
            url: self.url.clone(),
            license: self.license.clone(),
            install: self.install.clone(),
            changelog: self.changelog.clone(),
            groups: self.groups.clone(),
            arch: self.arch.clone(),
            backup: self.backup.clone(),
            depends: self.depends.clone(),
            optdepends: self.optdepends.clone(),
            conflicts: self.conflicts.clone(),
            provides: self.provides.clone(),
            replaces: self.replaces.clone(),
            options: self.options.clone(),
            overridden: HashSet::new(),
        }
    }
}

impl Package {
    pub fn is_overridden(&self, name: &str, arch: Option<&str>) -> bool {
        let key = Key {
            name: name.to_string(),
            arch: arch.map(|s| s.to_string()),
        };
        self.overridden.contains(&key)
    }
}

fn set_override_flag(package: &mut Package, var: &Variable) {
    package.overridden.insert(Key {
        name: var.name.clone(),
        arch: var.arch.clone(),
    });
}

impl Config {
    pub fn package_list(&self, pkgbuild: &Pkgbuild) -> Result<Vec<PathBuf>> {
        let dirs = self.pkgbuild_dirs(pkgbuild)?;
        let pkgbase = &pkgbuild.pkgbase;
        let version = pkgbuild.version();
        let mut pkgs = Vec::new();

        for p in pkgbuild.packages() {
            let filename = format!("{}-{}-{}{}", p.pkgname, version, self.arch, self.pkgext);
            pkgs.push(dirs.pkgdest.join(filename));

            if self.option(pkgbuild, "debug").enabled() && self.option(pkgbuild, "strip").enabled()
            {
                let filename = format!(
                    "{}-{}-{}-{}{}",
                    pkgbase, "debug", version, self.arch, self.pkgext
                );
                pkgs.push(dirs.pkgdest.join(filename));
            }
        }

        Ok(pkgs)
    }
}

#[cfg(test)]
mod test {
    use std::io::{stdout, Write};

    use ansi_term::{Color, Style};

    use crate::{CallBacks, Event, LogLevel, LogMessage, Makepkg, Options};

    use super::*;

    #[derive(Debug)]
    pub struct PrettyPrinter;

    impl CallBacks for PrettyPrinter {
        fn event(&mut self, event: Event) {
            match event {
                Event::FoundSource(_)
                | Event::Downloading(_)
                | Event::DownloadingCurl(_)
                | Event::NoExtact(_)
                | Event::Extacting(_)
                | Event::RemovingSrcdir
                | Event::RemovingPkgdir
                | Event::AddingFileToPackage(_)
                | Event::GeneratingPackageFile(_)
                | Event::DownloadingVCS(_, _)
                | Event::ExtractingVCS(_, _)
                | Event::UpdatingVCS(_, _) => println!("    {}", event),
                Event::VerifyingChecksum(_) | Event::VerifyingSignature(_) => {
                    print!("    {} ...", event);
                    let _ = stdout().flush();
                }
                Event::ChecksumSkipped(_)
                | Event::ChecksumFailed(_, _)
                | Event::ChecksumPass(_)
                | Event::SignatureCheckFailed(_)
                | Event::SignatureCheckPass(_) => println!(" {}", event),
                _ => {
                    println!(
                        "{} {}",
                        Style::new().bold().fg(Color::Cyan).paint("::"),
                        Style::new().bold().paint(event.to_string())
                    );
                }
            }
        }

        fn log(&mut self, level: LogLevel, msg: LogMessage) {
            match level {
                LogLevel::Error => println!(
                    "{}: {}",
                    Style::new().bold().fg(Color::Red).paint(level.to_string()),
                    msg
                ),
                LogLevel::Warning => println!(
                    "{}: {}",
                    Style::new()
                        .bold()
                        .fg(Color::Yellow)
                        .paint(level.to_string()),
                    msg
                ),
                _ => (),
            }
        }
    }

    #[ignore]
    #[test]
    fn geninteg() {
        let config = Makepkg::new().unwrap().callback(PrettyPrinter);
        let mut options = Options::new();
        options.clean_build = true;
        options.recreate_package = true;
        options.ignore_arch = true;
        let pkgbuild = Pkgbuild::new("../makepkg-test").unwrap();
        let res = config.geninteg(&options, &pkgbuild).unwrap();
        println!("{}", res);
    }

    #[test]
    fn lint_pkgbuild() {
        let makepkg = Makepkg::new().unwrap().callback(PrettyPrinter);
        let mut options = Options::new();
        options.clean_build = true;
        options.recreate_package = true;
        options.ignore_arch = true;
        options.no_build = true;
        let mut pkgbuild = Pkgbuild::new("../makepkg-test").unwrap();
        println!("{}", makepkg.geninteg(&options, &pkgbuild).unwrap());
        for pkg in makepkg.config.package_list(&pkgbuild).unwrap() {
            println!(" --- {}", pkg.display());
        }
        //let res = config.build(&options, &mut pkgbuild);
        let res = makepkg.build(&options, &mut pkgbuild);

        match res {
            Ok(_) => (),
            Err(err) => {
                print!(
                    "{}: {}",
                    Style::new().bold().fg(Color::Red).paint("error"),
                    err
                );
                if matches!(err, Error::AlreadyBuilt(_)) {
                    print!(" (use -f to overwrite)");
                }
                println!();
            }
        }
    }
}
