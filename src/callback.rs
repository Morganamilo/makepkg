use std::{
    fmt::Display,
    fs::File,
    io::{self, stdout, Write},
};

use crate::{
    error::{Context, IOContext, IOErrorExt, Result},
    pkgbuild::{Pkgbuild, Source},
    sources::VCSKind,
    Makepkg,
};

pub trait Callbacks: std::fmt::Debug + 'static {
    fn event(&mut self, _event: Event) -> io::Result<()> {
        Ok(())
    }
    fn log(&mut self, _level: LogLevel, _msg: LogMessage) -> io::Result<()> {
        Ok(())
    }

    fn command_new(&mut self, _id: usize, _kind: CommandKind) -> io::Result<CommandOutput> {
        Ok(Default::default())
    }
    fn command_exit(&mut self, _id: usize, _kind: CommandKind) -> io::Result<()> {
        Ok(())
    }
    fn command_output(&mut self, _id: usize, _kind: CommandKind, _output: &[u8]) -> io::Result<()> {
        Ok(())
    }

    fn download(&mut self, _pkgbuild: &Pkgbuild, _event: DownloadEvent) -> io::Result<()> {
        Ok(())
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Download<'a> {
    pub n: usize,
    pub total: usize,
    pub source: &'a Source,
}

#[derive(Debug, Copy, Clone, PartialEq, PartialOrd)]
pub enum DownloadEvent<'a> {
    DownloadStart(usize),
    Init(Download<'a>),
    Progress(Download<'a>, f64, f64),
    Completed(Download<'a>),
    Failed(Download<'a>, u32),
    DownloadEnd,
}

#[derive(Debug, Default)]
pub enum CommandOutput {
    #[default]
    Inherit,
    Null,
    Callback,
    File(File),
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CommandKind<'a> {
    PkgbuildFunction(&'a Pkgbuild),
    BuildingPackage(&'a Pkgbuild),
    DownloadSources(&'a Pkgbuild, &'a Source),
    ExtractSources(&'a Pkgbuild, &'a Source),
    Integ(&'a Pkgbuild, &'a Source),
}

impl<'a> CommandKind<'a> {
    pub fn pkgbuild(&self) -> &'a Pkgbuild {
        match self {
            CommandKind::PkgbuildFunction(p) => p,
            CommandKind::BuildingPackage(p) => p,
            CommandKind::DownloadSources(p, _) => p,
            CommandKind::ExtractSources(p, _) => p,
            CommandKind::Integ(p, _) => p,
        }
    }
}

#[derive(Debug)]
pub struct CallBackPrinter;

impl Callbacks for CallBackPrinter {
    fn event(&mut self, event: Event) -> io::Result<()> {
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
            | Event::UpdatingVCS(_, _) => writeln!(stdout(), "    {}", event),
            Event::VerifyingChecksum(_) | Event::VerifyingSignature(_) => {
                write!(stdout(), "    {} ...", event)?;
                stdout().flush()
            }
            Event::ChecksumSkipped(_)
            | Event::ChecksumFailed(_, _)
            | Event::ChecksumPass(_)
            | Event::SignatureCheckFailed(_)
            | Event::SignatureCheckPass(_) => writeln!(stdout(), " {}", event),
            _ => writeln!(stdout(), ":: {}", event),
        }
    }

    fn log(&mut self, level: LogLevel, msg: LogMessage) -> io::Result<()> {
        writeln!(stdout(), "{}: {}", level, msg)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SigFailedKind<'a> {
    NotSigned,
    UnknownPublicKey,
    Revoked,
    Expired,
    NotTrusted,
    NotInValidPgpKeys,
    Other(&'a str),
}

impl<'a> Display for SigFailedKind<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SigFailedKind::NotSigned => write!(f, "not signed"),
            SigFailedKind::UnknownPublicKey => write!(f, "unknown public key"),
            SigFailedKind::Revoked => f.write_str("key revoked"),
            SigFailedKind::Expired => f.write_str("key expired"),
            SigFailedKind::NotTrusted => f.write_str("not trusted"),
            SigFailedKind::NotInValidPgpKeys => f.write_str("not in validpgpkeys"),
            SigFailedKind::Other(e) => e.fmt(f),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SigFailed<'a> {
    pub file_name: &'a str,
    pub fingerprint: &'a str,
    pub kind: SigFailedKind<'a>,
}

impl<'a> Display for SigFailed<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} {}", self.kind, self.fingerprint)
    }
}

impl<'a> SigFailed<'a> {
    pub(crate) fn new(file_name: &'a str, fingerprint: &'a str, kind: SigFailedKind<'a>) -> Self {
        SigFailed {
            file_name,
            fingerprint,
            kind,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event<'a> {
    BuildingPackage(&'a str, &'a str),
    BuildingSourcePackage(&'a str, &'a str),
    BuiltPackage(&'a str, &'a str),
    BuiltSourcePackage(&'a str, &'a str),
    CreatingArchive(&'a str),
    RetrievingSources,
    FoundSource(&'a str),
    Downloading(&'a str),
    DownloadingCurl(&'a str),
    VerifyingSignatures,
    VerifyingChecksums,
    VerifyingSignature(&'a str),
    VerifyingChecksum(&'a str),
    ChecksumSkipped(&'a str),
    ChecksumFailed(&'a str, &'a [&'a str]),
    ChecksumPass(&'a str),
    SignatureCheckFailed(SigFailed<'a>),
    SignatureCheckPass(&'a str),
    ExtractingSources,
    GeneratingChecksums,
    SourcesAreReady,
    NoExtact(&'a str),
    Extacting(&'a str),
    RunningFunction(&'a str),
    RemovingSrcdir,
    RemovingPkgdir,
    UsingExistingSrcdir,
    StartingFakeroot,
    CreatingPackage(&'a str),
    CreatingDebugPackage(&'a str),
    CreatingSourcePackage(&'a str),
    AddingPackageFiles,
    AddingFileToPackage(&'a str),
    GeneratingPackageFile(&'a str),
    DownloadingVCS(VCSKind, &'a Source),
    UpdatingVCS(VCSKind, &'a Source),
    ExtractingVCS(VCSKind, &'a Source),
}

impl<'a> From<SigFailed<'a>> for Event<'a> {
    fn from(value: SigFailed<'a>) -> Self {
        Event::SignatureCheckFailed(value)
    }
}

impl<'a> Display for Event<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Event::BuildingPackage(name, ver) => write!(f, "Package {}-{}", name, ver),
            Event::BuildingSourcePackage(name, ver) => write!(f, "Source package {}-{}", name, ver),
            Event::BuiltPackage(name, ver) => write!(f, "Built package {}-{}", name, ver),
            Event::BuiltSourcePackage(name, ver) => {
                write!(f, "Built source package {}-{}", name, ver)
            }
            Event::CreatingArchive(file) => write!(f, "Creating Archive {}...", file),
            Event::AddingPackageFiles => write!(f, "Adding package files..."),
            Event::RetrievingSources => write!(f, "Retrieving sources..."),
            Event::VerifyingSignatures => write!(f, "Verifying source signatures..."),
            Event::VerifyingChecksums => write!(f, "Verifying source checksums..."),
            Event::FoundSource(file) => write!(f, "found {}", file),
            Event::Downloading(file) => write!(f, "downloading {}...", file),
            Event::DownloadingCurl(file) => write!(f, "downloading {}...", file),
            Event::VerifyingSignature(s) => write!(f, "{}", s),
            Event::VerifyingChecksum(s) => write!(f, "{}", s),
            Event::ChecksumSkipped(_) => write!(f, "Skipped"),
            Event::ChecksumFailed(_, v) => write!(f, "Failed ({})", v.join(" ")),
            Event::ChecksumPass(_) => write!(f, "Passsed"),
            Event::SignatureCheckFailed(e) => write!(f, "Failed ({})", e),
            Event::SignatureCheckPass(_) => write!(f, "Passsed"),
            Event::GeneratingChecksums => write!(f, "Generating checksums for source files..."),
            Event::ExtractingSources => write!(f, "ExtractingSources..."),
            Event::SourcesAreReady => write!(f, "Sources are ready"),
            Event::NoExtact(file) => write!(f, "skipping {} (no extract)", file),
            Event::Extacting(file) => write!(f, "extracting {} ...", file),
            Event::RunningFunction(func) => write!(f, "Starting {}()...", func),
            Event::RemovingSrcdir => write!(f, "removing existing $srcdir/ directory"),
            Event::RemovingPkgdir => write!(f, "removing existing $pkgdir/ directory"),
            Event::UsingExistingSrcdir => write!(f, "using existing $srcdir/ directory"),
            Event::StartingFakeroot => write!(f, "Starting fakeroot daemon..."),
            Event::CreatingPackage(file) => write!(f, "Creating package {}...", file),
            Event::CreatingDebugPackage(file) => write!(f, "Creating debug package {}...", file),
            Event::CreatingSourcePackage(file) => write!(f, "Creating source package {}...", file),
            Event::AddingFileToPackage(file) => write!(f, "adding {} ...", file),
            Event::GeneratingPackageFile(file) => write!(f, "generating {} ...", file),
            Event::DownloadingVCS(k, s) => write!(f, "cloning {} repo {} ...", k, s.file_name()),
            Event::UpdatingVCS(k, s) => write!(f, "updading {} repo {} ...", k, s.file_name()),
            Event::ExtractingVCS(k, s) => write!(
                f,
                "creating working copy of {} {} repo...",
                s.file_name(),
                k,
            ),
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LogLevel {
    Debug,
    Warning,
    Error,
}

impl Display for LogLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LogLevel::Debug => f.write_str("debug"),
            LogLevel::Warning => f.write_str("warning"),
            LogLevel::Error => f.write_str("error"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LogMessage<'a> {
    SkippingAllIntegrityChecks,
    SkippingPGPIntegrityChecks,
    SkippingChecksumIntegrityChecks,
    KeyNotDoundInKeys(&'a str),
}

impl<'a> Display for LogMessage<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LogMessage::SkippingAllIntegrityChecks => f.write_str("skipping all integrity checks"),
            LogMessage::SkippingPGPIntegrityChecks => {
                f.write_str("skipping signature integrity checks")
            }
            LogMessage::SkippingChecksumIntegrityChecks => {
                f.write_str("skipping checksum integrity checks")
            }
            LogMessage::KeyNotDoundInKeys(k) => write!(f, "key {} not found in keys/pgp", k),
        }
    }
}

impl Makepkg {
    pub fn event(&self, event: Event) -> Result<()> {
        if let Some(cb) = &mut *self.callbacks.borrow_mut() {
            cb.event(event)
                .context(Context::Callback, IOContext::WriteBuffer)?;
        }
        Ok(())
    }

    pub fn log(&self, level: LogLevel, msg: LogMessage) -> Result<()> {
        if let Some(cb) = &mut *self.callbacks.borrow_mut() {
            cb.log(level, msg)
                .context(Context::Callback, IOContext::WriteBuffer)?;
        }
        Ok(())
    }

    pub fn download(&self, pkgbuild: &Pkgbuild, event: DownloadEvent) -> Result<()> {
        if let Some(cb) = &mut *self.callbacks.borrow_mut() {
            cb.download(pkgbuild, event)
                .context(Context::Callback, IOContext::WriteBuffer)?;
        }
        Ok(())
    }
}
