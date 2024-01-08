use std::io::ErrorKind;
use std::process::{Child, ExitStatus, Output};
use std::{
    fmt::Display,
    io, iter,
    path::{PathBuf, StripPrefixError},
    process::Command,
    result::Result as StdResult,
    string::FromUtf8Error,
    time::SystemTimeError,
};

use crate::{
    package::PackageKind,
    pkgbuild::{Fragment, Source},
    sources::VCSKind,
    FileKind,
};

pub type Result<T> = std::result::Result<T, Error>;

pub(crate) trait CommandErrorExt<T>: Sized {
    fn cmd_context(self, command: &Command, context: Context) -> StdResult<T, CommandError>;
    fn download_context(
        self,
        source: &Source,
        cmd: &Command,
        context: Context,
    ) -> StdResult<T, DownloadError> {
        self.cmd_context(cmd, context)
            .map_err(|e| DownloadError::Command(source.clone(), e))
    }
}

pub(crate) trait IOErrorExt<T> {
    fn context(self, context: Context, iocontext: IOContext) -> StdResult<T, IOError>;
}

impl<T> CommandErrorExt<T> for StdResult<T, FromUtf8Error> {
    fn cmd_context(self, command: &Command, context: Context) -> StdResult<T, CommandError> {
        self.map_err(|e| CommandError::utf8(e, command, context))
    }
}

impl CommandErrorExt<Child> for io::Result<Child> {
    fn cmd_context(self, command: &Command, context: Context) -> StdResult<Child, CommandError> {
        self.map_err(|e| CommandError::exec(e, command, context))
    }
}

impl CommandErrorExt<Output> for io::Result<Output> {
    fn cmd_context(self, command: &Command, context: Context) -> StdResult<Output, CommandError> {
        match self {
            Ok(status) if !status.status.success() => {
                Err(CommandError::exit(command, status.status.code(), context))
            }
            Ok(o) => Ok(o),
            Err(e) => Err(CommandError::exec(e, command, context)),
        }
    }
}

impl CommandErrorExt<()> for io::Result<()> {
    fn cmd_context(self, command: &Command, context: Context) -> StdResult<(), CommandError> {
        self.map_err(|e| CommandError::exec(e, command, context))
    }
}

impl CommandErrorExt<ExitStatus> for io::Result<ExitStatus> {
    fn cmd_context(
        self,
        command: &Command,
        context: Context,
    ) -> StdResult<ExitStatus, CommandError> {
        match self {
            Ok(status) if !status.success() => {
                Err(CommandError::exit(command, status.code(), context))
            }
            Ok(o) => Ok(o),
            Err(e) => Err(CommandError::exec(e, command, context)),
        }
    }
}

impl<T> IOErrorExt<T> for nix::Result<T> {
    fn context(self, context: Context, iocontext: IOContext) -> StdResult<T, IOError> {
        self.map_err(|e| io::Error::from_raw_os_error(e as i32))
            .context(context, iocontext)
    }
}

impl<T> IOErrorExt<T> for io::Result<T> {
    fn context(self, context: Context, iocontext: IOContext) -> StdResult<T, IOError> {
        self.map_err(|e| IOError::new(context, iocontext, e))
    }
}

impl<T> IOErrorExt<T> for StdResult<T, StripPrefixError> {
    fn context(self, context: Context, iocontext: IOContext) -> StdResult<T, IOError> {
        self.map_err(|e| {
            IOError::new(
                context,
                iocontext,
                io::Error::new(io::ErrorKind::NotFound, e),
            )
        })
    }
}

impl<T> IOErrorExt<T> for walkdir::Result<T> {
    fn context(self, context: Context, iocontext: IOContext) -> StdResult<T, IOError> {
        self.map_err(io::Error::from).context(context, iocontext)
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct DownloadAgentError {
    pub input: String,
}

impl Display for DownloadAgentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.input.is_empty() {
            write!(f, "DLAGENT is empty")?;
        }

        write!(f, "invalid DLAGENT \"{}\" (no protocol)", self.input)
    }
}

impl std::error::Error for DownloadAgentError {}

#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct VCSClientError {
    pub input: String,
}

impl Display for VCSClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "invalid VCS client \"{}\"", self.input)
    }
}

impl std::error::Error for VCSClientError {}

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Expected {
    String,
    Array,
}

impl Display for Expected {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Expected::String => f.write_str("string"),
            Expected::Array => f.write_str("array"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseErrorKind {
    UnknownEscapeSequence(char),
    UnterminatedString(String),
    UnescapedQuoteInString(String),
    UnexpectedWord(String),
    UnexpectedEndOfInput,
}

impl Display for ParseErrorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParseErrorKind::UnknownEscapeSequence(c) => {
                write!(f, "unknown escape sequence '\\{}'", c)
            }
            ParseErrorKind::UnterminatedString(word) => write!(f, "unterminated string: {}", word),
            ParseErrorKind::UnescapedQuoteInString(word) => {
                write!(f, "unescaped '\"' in quoted string: {}", word)
            }
            ParseErrorKind::UnexpectedWord(word) => write!(f, "unexpected word {}", word),
            ParseErrorKind::UnexpectedEndOfInput => f.write_str("unexpected end of input"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub line: String,
    pub kind: ParseErrorKind,
    pub file_kind: FileKind,
}

impl Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "failed to parse {}: {}", self.file_kind, self.kind)
    }
}

impl ParseError {
    pub(crate) fn new<S: Into<String>>(line: S, file_kind: FileKind, kind: ParseErrorKind) -> Self {
        Self {
            line: line.into(),
            file_kind,
            kind,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Context {
    IntegrityCheck,
    RetrieveSources,
    ExtractSources,
    GenerateSrcinfo,
    SetPkgbuildVar(String),
    UnifySourceTime,
    CreatePackage,
    BuildPackage,
    GetPackageSize,
    GetPackageFiles,
    GeneratePackageFile(String),
    RunFunction(String),
    ReadPkgbuild,
    SourcePkgbuild,
    ParsePkgbuild,
    ReadConfig,
    QueryPacman,
    RunPacman,
    StartFakeroot,
    None,
}

impl Display for Context {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Context::IntegrityCheck => f.write_str("failed to validate sources"),
            Context::RetrieveSources => f.write_str("failed to download sources"),
            Context::ExtractSources => f.write_str("failed to extract sources"),
            Context::GenerateSrcinfo => f.write_str("failed to generate .SRCINFO"),
            Context::SetPkgbuildVar(v) => write!(f, "failed to set {}", v),
            Context::UnifySourceTime => write!(f, "failed to unify file timestamps"),
            Context::CreatePackage => write!(f, "failed to create package tarball"),
            Context::BuildPackage => write!(f, "failed to build package"),
            Context::GetPackageSize => write!(f, "failed to get packge size"),
            Context::GetPackageFiles => write!(f, "failed to get packge files"),
            Context::GeneratePackageFile(name) => write!(f, "failed to generate {}", name),
            Context::RunFunction(func) => write!(f, "failed to run {}()", func),
            Context::ReadPkgbuild => write!(f, "failed to read pkgbuild"),
            Context::SourcePkgbuild => write!(f, "failed to source pkgbuild"),
            Context::ParsePkgbuild => write!(f, "failed to parse pkgbuild"),
            Context::ReadConfig => write!(f, "failed to read config file"),
            Context::QueryPacman => write!(f, "failed to query pacman"),
            Context::RunPacman => write!(f, "failed to run pacman"),
            Context::StartFakeroot => write!(f, "failed to start fakeroot"),
            Context::None => f.write_str("no context"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum IOContext {
    HashFile(PathBuf),
    WriteDownload(String),
    WriteStdout,
    Mkdir(PathBuf),
    Open(PathBuf),
    Write(PathBuf),
    Read(PathBuf),
    ReadDir(PathBuf),
    CurrentDir,
    Rename(PathBuf, PathBuf),
    Utimensat(PathBuf),
    RemoveTempfile(PathBuf),
    Remove(PathBuf),
    MakeLink(PathBuf, PathBuf),
    ReadLink(PathBuf),
    Copy(PathBuf, PathBuf),
    WriteProcess(String),
    Stat(PathBuf),
    Pipe,
    Dup,
    InvalidPath(PathBuf),
    NotAFile(PathBuf),
    NotADir(PathBuf),
    NotFound(PathBuf),
    FindLibfakeroot(Vec<PathBuf>),
    Chmod(PathBuf),
}

impl Display for IOContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IOContext::HashFile(p) => write!(f, "unable to hash source {}", p.display()),
            IOContext::WriteDownload(p) => {
                write!(f, "unable to write to download file  {}", p)
            }
            IOContext::WriteStdout => write!(f, "unable to write to stdout"),
            IOContext::Mkdir(p) => write!(f, "mkdir {}", p.display()),
            IOContext::Open(p) => write!(f, "open {}", p.display()),
            IOContext::Write(p) => write!(f, "write {}", p.display()),
            IOContext::Read(p) => write!(f, "read {}", p.display()),
            IOContext::ReadDir(p) => write!(f, "read dir {}", p.display()),
            IOContext::CurrentDir => write!(f, "failed to get current directory"),
            IOContext::Rename(src, dst) => {
                write!(f, "rename {} -> {}", src.display(), dst.display())
            }
            IOContext::Utimensat(p) => write!(f, "failed to change access time: {}", p.display()),
            IOContext::RemoveTempfile(p) => write!(f, "can't remove tempfile {}", p.display()),
            IOContext::Remove(p) => write!(f, "rm {}", p.display()),
            IOContext::MakeLink(src, dst) => {
                write!(f, "link {} -> {}", dst.display(), src.display())
            }
            IOContext::ReadLink(p) => write!(f, "readlink {}", p.display()),
            IOContext::Copy(src, dst) => write!(f, "copy {} -> {}", src.display(), dst.display()),
            IOContext::WriteProcess(name) => write!(f, "couldn't write to {}", name),
            IOContext::Stat(p) => write!(f, "stat {}", p.display()),
            IOContext::Pipe => write!(f, "unable to create pipe"),
            IOContext::Dup => write!(f, "unable to duplicate file description"),
            IOContext::InvalidPath(p) => write!(f, "invalid path \"{}\"", p.display()),
            IOContext::NotAFile(p) => write!(f, "{} is not a file", p.display()),
            IOContext::NotADir(p) => write!(f, "{} is not a directory", p.display()),
            IOContext::NotFound(p) => write!(f, "{}: no such file or directory", p.display()),
            IOContext::Chmod(p) => write!(f, "can't change permissions on {}", p.display()),
            IOContext::FindLibfakeroot(p) => {
                write!(f, "can't find fakeroot library (searched:",)?;
                for p in p {
                    write!(f, " {}", p.display())?;
                }
                write!(f, ")")
            }
        }
    }
}

#[derive(Debug)]
pub struct IOError {
    pub context: Context,
    pub iocontext: IOContext,
    pub err: std::io::Error,
}

impl std::error::Error for IOError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.err)
    }
}

impl Display for IOError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.context != Context::None {
            write!(f, "{}", self.context)?;
        }
        write!(f, ": {}", self.iocontext)?;

        if self.err.kind() != ErrorKind::Other {
            write!(f, ": {}", self.err)?;
        }
        Ok(())
    }
}

impl IOError {
    pub(crate) fn new<E: Into<io::Error>>(context: Context, iocontext: IOContext, err: E) -> Self {
        IOError {
            context,
            iocontext,
            err: err.into(),
        }
    }
}

#[derive(Debug)]
pub enum CommandErrorKind {
    Command(io::Error),
    UTF8(FromUtf8Error),
    ExitCode(Option<i32>),
}

impl Display for CommandErrorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CommandErrorKind::Command(e) => e.fmt(f),
            CommandErrorKind::UTF8(_) => write!(f, "output was not valid unicode"),
            CommandErrorKind::ExitCode(Some(code)) => write!(f, "exited {}", code),
            CommandErrorKind::ExitCode(None) => write!(f, "\" killed by signal"),
        }
    }
}

#[derive(Debug, Clone)]
pub enum LintKind {
    UnknownFragment(String),
    WrongValueType(String, String, String),
    CantBeArchitectureSpecific(String, String),
    CantBeArchitectureSpecificAny,
    VariableCantBeInPackageFunction(String),
    VariabeContainsNewlines(String),
    VariabeContainsEmptyString(String),
    ConflictingPackageFunctions,
    WrongPackgeFunctionFormat,
    MissingPackageFunction(String),
    MissingFile(String, String),
    AnyArchWithOthers,
    BackupHasLeadingSlash(String),
    IntegrityChecksMissing(String),
    StartsWithInvalid(String, String),
    InvalidChars(String, String),
    InvalidPkgver(String),
    InvalidPkgrel(String),
    AsciiOnly(String, String),
    IntegrityChecksDifferentSize(String, String),
    InvalidPkgExt(String),
    InvalidSrcExt(String),
    InvalidEpoch(String),
    InvalidVCSClient(VCSClientError),
    InvalidDownloadAgent(DownloadAgentError),
    InvalidSystemTime(SystemTimeError),
}

impl Display for LintKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LintKind::CantBeArchitectureSpecific(v, a) => {
                write!(f, "{} can not be architecture specific {}", v, a)
            }
            LintKind::CantBeArchitectureSpecificAny => write!(
                f,
                "can't provide architecture specific variables for the 'any' architecture"
            ),
            LintKind::VariableCantBeInPackageFunction(v) => write!(f, "{} can not be set inside of package()", v),
            LintKind::VariabeContainsNewlines(v) => write!(f, "{} does not allow new lines", v),
            LintKind::VariabeContainsEmptyString(v) => write!(f, "{} does not allow empty values", v),
            LintKind::ConflictingPackageFunctions => write!(f, "conflicting package function: 'package' and 'package_%$pkgname' functions can not be used together"),
            LintKind::WrongPackgeFunctionFormat => write!(f, "when building split packages the package functions must be in the form 'package_$pkgname'"),
            LintKind::MissingPackageFunction(v) => write!(f, "missing packge function for {}", v),
            LintKind::MissingFile(n, v) => write!(f, "{} file '{}' does not exist", n, v),
            LintKind::AnyArchWithOthers => write!(f, "can't use the any architecture with other architectures"),
            LintKind::BackupHasLeadingSlash(b) => write!(f, "backup entry should not contain a leading slash: '{}'", b),
            LintKind::IntegrityChecksMissing(v) => write!(f, "integrity checks are missing for {}", v),
            LintKind::StartsWithInvalid(k, c) => write!(f, "{} is not allowed to start with '{}'", k, c),
            LintKind::InvalidChars(k, c) => write!(f, "{} contains invalid characters '{}'", k, c),
            LintKind::InvalidPkgver(v) => write!(f, "pkgver in {} is not allowed to contain colons, forward slashes. hyphens or whitespace", v),
            LintKind::InvalidPkgrel(v) => write!(f, "pkgrel must be in the form integral[.integer] not '{}'", v),
            LintKind::AsciiOnly(k, v) => write!(f, "{} in {} is only allowd to contain ascii", k, v),
            LintKind::IntegrityChecksDifferentSize(k, v) => write!(f, "integrity check {} differs in size from {}", k, v),
            LintKind::UnknownFragment(fragment) => write!(f, "invalid fragment '{}'", fragment),
            LintKind::WrongValueType(name, expected, got) => write!(f, "{}: expected {} got {}", name, expected, got),
            LintKind::InvalidPkgExt(_) => {
                write!(f, "PKGEXT is invalid: PKGEXT must begin with .pkg.tar")
            }
            LintKind::InvalidSrcExt(_) => {
                write!(f, "SRCEXT is invalid: SRCEXT must begin with .src.tar")
            }
            LintKind::InvalidEpoch(e) => {
                write!(f, "SOURCE_DATE_EPOCH '{}' is not a number", e)
            }
            LintKind::InvalidVCSClient(e) => e.fmt(f),
            LintKind::InvalidDownloadAgent(e) => e.fmt(f),
            LintKind::InvalidSystemTime(_) => f.write_str("invalid system time"),
        }
    }
}

impl LintKind {
    pub(crate) fn pkgbuild(self) -> LintError {
        LintError::pkgbuild(vec![self])
    }

    pub(crate) fn config(self) -> LintError {
        LintError::config(vec![self])
    }
}

#[derive(Debug, Clone)]
pub struct LintError {
    file_kind: FileKind,
    issues: Vec<LintKind>,
}

impl Display for LintError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.file_kind {
            FileKind::Pkgbuild => f.write_str("invalid PKGBUILD: ")?,
            FileKind::Config => f.write_str("invalid config")?,
        }
        if let Some(issue) = self.issues.get(0) {
            issue.fmt(f)?;
        }
        for issue in self.issues.iter().skip(1) {
            f.write_str("\n    ")?;
            issue.fmt(f)?;
        }
        Ok(())
    }
}

impl LintError {
    pub(crate) fn pkgbuild(v: Vec<LintKind>) -> Self {
        LintError {
            file_kind: FileKind::Pkgbuild,
            issues: v,
        }
    }
    pub(crate) fn config(v: Vec<LintKind>) -> Self {
        LintError {
            file_kind: FileKind::Config,
            issues: v,
        }
    }
}

#[derive(Debug)]
pub enum DownloadError {
    SourceMissing(Source),
    UnknownProtocol(Source),
    UnknownVCSClient(Source),
    Curl(curl::Error),
    CurlMulti(curl::MultiError),
    Status(Source, u32),
    Command(Source, CommandError),
    UnsupportedFragment(Source, VCSKind, Fragment),
    RemotesDiffer(Source, String),
    RefsDiffer(Source, String, String),
    NotCheckedOut(Source),
}

impl Display for DownloadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("failed to retrieve sources: ")?;
        match self {
            DownloadError::SourceMissing(s) => write!(f, "can't find source {}", s),
            DownloadError::UnknownProtocol(s) => write!(f, "unknown protocol {}", s),
            DownloadError::UnknownVCSClient(s) => write!(f, "unknown VCS client {}", s),
            DownloadError::Curl(e) => write!(f, "curl: {}", e),
            DownloadError::CurlMulti(e) => write!(f, "curl: {}", e),
            DownloadError::Status(s, code) => write!(f, "{} (status {})", s.file_name(), code),
            DownloadError::Command(s, e) => write!(f, "{} ({})", s.file_name(), e),
            DownloadError::RemotesDiffer(s, _) => {
                write!(f, "{} is not a clone of {}", s.file_name(), s.url)
            }
            DownloadError::UnsupportedFragment(s, k, frag) => {
                write!(f, "{}: {} does not support fragment {}", s, k, frag.kind())
            }
            DownloadError::RefsDiffer(s, r, _) => {
                write!(
                    f,
                    "{}: failed to checkout version {}, the git tag has been forged",
                    s.file_name(),
                    r,
                )
            }
            DownloadError::NotCheckedOut(s) => write!(f, "{} is not checked out", s.file_name()),
        }
    }
}

#[derive(Debug)]
pub enum IntegError {
    ValidityCheck,
    VerifyFunction,
    MissingFileForSig(String),
    ReadFingerprint(String),
    Gpgme(gpgme::Error),
}

impl Display for IntegError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IntegError::ValidityCheck => {
                f.write_str("one or more files did not pass the validity check")
            }
            IntegError::VerifyFunction => {
                f.write_str("verify() function failed to validate sources")
            }
            IntegError::MissingFileForSig(s) => {
                write!(f, "signature {} has no accompanying file", s)
            }
            IntegError::ReadFingerprint(s) => {
                write!(f, "failed to get fingerprint for {}", s)
            }
            IntegError::Gpgme(e) => {
                write!(f, "gpgme: {}", e)
            }
        }
    }
}

#[derive(Debug)]
pub struct CommandError {
    pub kind: CommandErrorKind,
    pub command: Vec<String>,
    pub context: Context,
}

impl Display for CommandError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.context != Context::None {
            write!(f, "{}: ", self.context)?;
        }
        match &self.kind {
            CommandErrorKind::Command(_) => write!(f, "{} ({})", self.command[0], self.kind)?,
            CommandErrorKind::UTF8(_) => write!(f, "{}: {}", self.command[0], self.kind)?,
            CommandErrorKind::ExitCode(_) => write!(f, "{} {}", self.command[0], self.kind)?,
        }

        Ok(())
    }
}

impl CommandError {
    pub(crate) fn exec(err: io::Error, command: &Command, context: Context) -> Self {
        CommandError {
            command: Self::command_to_string(command),
            context,
            kind: CommandErrorKind::Command(err),
        }
    }
    pub(crate) fn utf8(err: FromUtf8Error, command: &Command, context: Context) -> Self {
        CommandError {
            command: Self::command_to_string(command),
            context,
            kind: CommandErrorKind::UTF8(err),
        }
    }
    pub(crate) fn exit(command: &Command, code: Option<i32>, context: Context) -> Self {
        CommandError {
            command: Self::command_to_string(command),
            context,
            kind: CommandErrorKind::ExitCode(code),
        }
    }

    fn command_to_string(command: &Command) -> Vec<String> {
        iter::once(command.get_program())
            .chain(command.get_args())
            .map(|s| s.to_string_lossy().to_string())
            .collect()
    }
}

#[derive(Debug, Clone)]
pub struct ArchitectureError {
    pub pkgbase: String,
    pub arch: String,
}

impl Display for ArchitectureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} is not avaliable for the {} architecture",
            self.pkgbase, self.arch
        )
    }
}

#[derive(Debug)]
pub struct AlreadyBuiltError {
    pub kind: PackageKind,
    pub pkgbase: String,
}

impl Display for AlreadyBuiltError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} has already been built", self.kind)
    }
}

#[derive(Debug)]
pub enum Error {
    Parse(ParseError),
    Lint(LintError),
    IO(IOError),
    Download(DownloadError),
    Integ(IntegError),
    Architecture(ArchitectureError),
    AlreadyBuilt(AlreadyBuiltError),
    Command(CommandError),
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::IO(e) => Some(&e.err as _),
            _ => None,
        }
    }
}

impl Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Parse(e) => e.fmt(f),
            Error::Lint(e) => e.fmt(f),
            Error::IO(e) => e.fmt(f),
            Error::Download(e) => e.fmt(f),
            Error::Integ(e) => e.fmt(f),
            Error::Architecture(e) => e.fmt(f),
            Error::AlreadyBuilt(e) => e.fmt(f),
            Error::Command(e) => e.fmt(f),
        }
    }
}

/*impl Error {
    pub fn context(&self) -> Context {
        match self {
            Error::Parse(_) => todo!(),
            Error::Lint(_) => todo!(),
            Error::IO(e) => e.context,
            Error::Download(_) => todo!(),
            Error::Integ(_) => todo!(),
            Error::Architecture(_) => todo!(),
            Error::AlreadyBuilt(_) => todo!(),
            Error::Command(_) => todo!(),
        }
    }
}*/

impl From<ParseError> for Error {
    fn from(value: ParseError) -> Self {
        Self::Parse(value)
    }
}

impl From<IOError> for Error {
    fn from(value: IOError) -> Self {
        Self::IO(value)
    }
}

impl From<LintError> for Error {
    fn from(value: LintError) -> Self {
        Self::Lint(value)
    }
}

impl From<DownloadError> for Error {
    fn from(value: DownloadError) -> Self {
        Self::Download(value)
    }
}

impl From<curl::Error> for Error {
    fn from(value: curl::Error) -> Self {
        DownloadError::Curl(value).into()
    }
}

impl From<curl::MultiError> for Error {
    fn from(value: curl::MultiError) -> Self {
        DownloadError::CurlMulti(value).into()
    }
}

impl From<IntegError> for Error {
    fn from(value: IntegError) -> Self {
        Error::Integ(value)
    }
}

impl From<CommandError> for Error {
    fn from(value: CommandError) -> Self {
        Error::Command(value)
    }
}

impl From<ArchitectureError> for Error {
    fn from(value: ArchitectureError) -> Self {
        Error::Architecture(value)
    }
}

impl From<AlreadyBuiltError> for Error {
    fn from(value: AlreadyBuiltError) -> Self {
        Error::AlreadyBuilt(value)
    }
}
