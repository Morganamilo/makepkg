use std::{
    cell::RefCell,
    fmt::Display,
    io::{stdout, Write},
};

use crate::{pkgbuild::Source, sources::VCSKind, Makepkg};

pub trait CallBacks: std::fmt::Debug {
    fn event(&mut self, _event: Event) {}
    fn progress(&mut self, _source: Source, _dltotal: f64, _dlnow: f64) {}
    fn log(&mut self, _level: LogLevel, _msg: LogMessage) {}
}

#[derive(Debug)]
pub struct CallBackPrinter;

impl CallBacks for CallBackPrinter {
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
            _ => println!(":: {}", event),
        }
    }

    fn log(&mut self, level: LogLevel, msg: LogMessage) {
        println!("{}: {}", level, msg);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SigFailedKind {
    NotSigned,
    UnknownPublicKey,
    Revoked,
    Expired,
    NotTrusted,
    NotInValidPgpKeys,
    Other(String),
}

impl Display for SigFailedKind {
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
pub struct SigFailed {
    pub file_name: String,
    pub fingerprint: String,
    pub kind: SigFailedKind,
}

impl Display for SigFailed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} {}", self.kind, self.fingerprint)
    }
}

impl SigFailed {
    pub(crate) fn new<S: Into<String>>(file_name: S, fingerprint: S, kind: SigFailedKind) -> Self {
        SigFailed {
            file_name: file_name.into(),
            fingerprint: fingerprint.into(),
            kind,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    BuildingPackage(String, String),
    BuildingSourcePackage(String, String),
    BuiltPackage(String, String),
    BuiltSourcePackage(String, String),
    RetrievingSources,
    FoundSource(String),
    Downloading(String),
    DownloadingCurl(String),
    VerifyingSignatures,
    VerifyingChecksums,
    VerifyingSignature(String),
    VerifyingChecksum(String),
    ChecksumSkipped(String),
    ChecksumFailed(String, Vec<String>),
    ChecksumPass(String),
    SignatureCheckFailed(SigFailed),
    SignatureCheckPass(String),
    ExtractingSources,
    GeneratingChecksums,
    SourcesAreReady,
    NoExtact(String),
    Extacting(String),
    RunningFunction(String),
    RemovingSrcdir,
    RemovingPkgdir,
    UsingExistingSrcdir,
    StartingFakeroot,
    CreatingPackage(String),
    CreatingDebugPackage(String),
    CreatingSourcePackage(String),
    AddingPackageFiles,
    AddingFileToPackage(String),
    GeneratingPackageFile(String),
    DownloadingVCS(VCSKind, Source),
    UpdatingVCS(VCSKind, Source),
    ExtractingVCS(VCSKind, Source),
}

impl From<SigFailed> for Event {
    fn from(value: SigFailed) -> Self {
        Event::SignatureCheckFailed(value)
    }
}

impl Display for Event {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Event::BuildingPackage(name, ver) => write!(f, "Package {}-{}", name, ver),
            Event::BuildingSourcePackage(name, ver) => write!(f, "Source package {}-{}", name, ver),
            Event::BuiltPackage(name, ver) => write!(f, "Built package {}-{}", name, ver),
            Event::BuiltSourcePackage(name, ver) => {
                write!(f, "Built source package {}-{}", name, ver)
            }
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
            Event::GeneratingChecksums => write!(f, "Generating checksums for source files"),
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

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LogMessage {
    SkippingAllIntegrityChecks,
    SkippingPGPIntegrityChecks,
    SkippingChecksumIntegrityChecks,
    KeyNotDoundInKeys(String),
}

impl Display for LogMessage {
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
    pub fn callback<CB: CallBacks + 'static>(mut self, callbacks: CB) -> Self {
        self.callbacks = Some(Box::new(RefCell::new(callbacks)));
        self
    }

    pub fn event(&self, event: Event) {
        if let Some(cb) = &self.callbacks {
            cb.borrow_mut().event(event)
        }
    }

    pub fn log(&self, level: LogLevel, msg: LogMessage) {
        if let Some(cb) = &self.callbacks {
            cb.borrow_mut().log(level, msg)
        }
    }

    pub fn progress(&self, source: Source, dltotal: f64, dlnow: f64) {
        if let Some(cb) = &self.callbacks {
            cb.borrow_mut().progress(source, dltotal, dlnow)
        }
    }
}
