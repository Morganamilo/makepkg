use std::fs::File;
use std::io::{ErrorKind, Read, Write};
use std::path::Path;

use blake2::Blake2b512;
use digest::Digest;
use gpgme::{Protocol, SignatureSummary, Validity};
use md5::Md5;
use sha1::Sha1;
use sha2::{Sha224, Sha256, Sha384, Sha512};

use crate::callback::{Event, LogLevel, LogMessage, SigFailed, SigFailedKind};
use crate::config::PkgbuildDirs;
use crate::error::{
    CommandError, CommandErrorKind, Context, Error, IOContext, IOErrorExt, IntegError, Result,
};
use crate::fs::open;
use crate::options::Options;
use crate::pkgbuild::{ArchVec, ArchVecs, ChecksumKind, Function, Pkgbuild, Source};
use crate::Makepkg;

mod bzr;
mod git;
mod mercurial;
mod vcs;

impl Makepkg {
    pub fn check_integ(&self, options: &Options, pkgbuild: &Pkgbuild, all: bool) -> Result<()> {
        if options.no_signatures && options.no_checksums {
            self.log(LogLevel::Warning, LogMessage::SkippingAllIntegrityChecks)?;
            return Ok(());
        }

        let dirs = self.pkgbuild_dirs(pkgbuild)?;

        if options.no_checksums {
            self.log(
                LogLevel::Warning,
                LogMessage::SkippingChecksumIntegrityChecks,
            )?;
            self.check_signatures(pkgbuild, all)?
        } else if options.no_signatures {
            self.log(LogLevel::Warning, LogMessage::SkippingPGPIntegrityChecks)?;
            self.check_checksums(&dirs, pkgbuild, all)?;
        } else {
            self.check_checksums(&dirs, pkgbuild, all)?;
            self.check_signatures(pkgbuild, all)?;
        }

        if pkgbuild.has_function(Function::Verify) {
            let err = self.run_function(options, pkgbuild, Function::Verify);
            if let Err(Error::Command(CommandError {
                kind: CommandErrorKind::ExitCode(Some(_)),
                ..
            })) = err
            {
                return Err(IntegError::VerifyFunction.into());
            }
        }

        Ok(())
    }

    pub fn check_signatures(&self, pkgbuild: &Pkgbuild, all: bool) -> Result<()> {
        self.event(Event::VerifyingSignatures)?;
        let mut gpg =
            gpgme::Context::from_protocol(Protocol::OpenPgp).map_err(IntegError::Gpgme)?;
        let mut ok = true;
        let dirs = self.pkgbuild_dirs(pkgbuild)?;

        for source in &pkgbuild.source.values {
            if !all && !source.enabled(&self.config.arch) {
                continue;
            }

            ok &= self.check_sigs_one_arch(&dirs, &mut gpg, pkgbuild, source)?;
        }

        if !ok {
            return Err(IntegError::ValidityCheck.into());
        }

        Ok(())
    }

    fn check_sigs_one_arch(
        &self,
        dirs: &PkgbuildDirs,
        gpg: &mut gpgme::Context,
        pkgbuild: &Pkgbuild,
        sources: &ArchVec<Source>,
    ) -> Result<bool> {
        let mut ok = true;

        for source in &sources.values {
            if let Some(proto) = source.vcs_kind() {
                ok &= self.verify_vcs_sig(dirs, proto, pkgbuild, source, gpg)?;
                continue;
            }

            let (file, ext) = match source.file_name().rsplit_once('.') {
                Some((file, ext)) => (file, ext),
                None => continue,
            };

            if ext != "asc" && ext != "sig" {
                continue;
            }

            let source_file = sources
                .values
                .iter()
                .find(|s| s.file_name() == file)
                .ok_or_else(|| IntegError::MissingFileForSig(source.file_name().to_string()))?;

            let sig = dirs.download_path(source);
            let data = dirs.download_path(source_file);
            let sig = open(File::options().read(true), sig, Context::IntegrityCheck)?;
            let data = open(File::options().read(true), data, Context::IntegrityCheck)?;

            let res = gpg.verify_detached(sig, data).map_err(IntegError::Gpgme)?;
            ok &= self.process_sig(source_file, pkgbuild, &res)?;
        }

        Ok(ok)
    }

    fn process_sig(
        &self,
        source: &Source,
        pkgbuild: &Pkgbuild,
        res: &gpgme::VerificationResult,
    ) -> Result<bool> {
        let mut ok = true;

        let file = source.file_name();
        self.event(Event::VerifyingSignature(file))?;

        for sig in res.signatures() {
            let fingerprint = sig
                .fingerprint()
                .map_err(|_| IntegError::ReadFingerprint(file.to_string()))?;
            if let Err(err) = sig.status() {
                ok = false;

                if sig.summary().contains(SignatureSummary::KEY_MISSING) {
                    self.event(
                        SigFailed::new(file, fingerprint, SigFailedKind::UnknownPublicKey).into(),
                    )?;
                } else if sig.summary().contains(SignatureSummary::KEY_REVOKED) {
                    self.event(SigFailed::new(file, fingerprint, SigFailedKind::Revoked).into())?;
                } else if sig.summary().contains(SignatureSummary::KEY_REVOKED) {
                    self.event(SigFailed::new(file, fingerprint, SigFailedKind::Expired).into())?;
                } else {
                    let d = err.to_string();
                    self.event(SigFailed::new(file, fingerprint, SigFailedKind::Other(&d)).into())?;
                }
                continue;
            }

            if pkgbuild.validpgpkeys.is_empty() {
                if !matches!(
                    sig.validity(),
                    Validity::Full | Validity::Marginal | Validity::Ultimate
                ) {
                    self.event(
                        SigFailed::new(file, fingerprint, SigFailedKind::NotTrusted).into(),
                    )?;
                    ok = false;
                }
            } else if !pkgbuild.validpgpkeys.iter().any(|p| p == fingerprint) {
                self.event(
                    SigFailed::new(file, fingerprint, SigFailedKind::NotInValidPgpKeys).into(),
                )?;
                ok = false;
            } else {
                self.event(Event::SignatureCheckPass(file))?
            }
        }

        Ok(ok)
    }

    pub fn check_checksums(
        &self,
        dirs: &PkgbuildDirs,
        pkgbuild: &Pkgbuild,
        all: bool,
    ) -> Result<()> {
        self.event(Event::VerifyingChecksums)?;

        let mut ok = true;

        for source in &pkgbuild.source.values {
            if !all && !source.enabled(&self.config.arch) {
                continue;
            }
            let sums = pkgbuild
                .get_all_checksums()
                .map(|(k, a)| (k, get_sum_array(a, &source.arch)));

            for (n, source) in source.values.iter().enumerate() {
                ok &= self.check_checksums_one_file(dirs, pkgbuild, source, n, sums)?;
            }
        }

        if !ok {
            return Err(IntegError::ValidityCheck.into());
        }

        Ok(())
    }

    fn check_checksums_one_file(
        &self,
        dirs: &PkgbuildDirs,
        p: &Pkgbuild,
        source: &Source,
        n: usize,
        sums: [(ChecksumKind, &[String]); ChecksumKind::len()],
    ) -> Result<bool> {
        let mut failed = Vec::new();
        self.event(Event::VerifyingChecksum(source.file_name()))?;

        if sums
            .iter()
            .filter_map(|(_, v)| v.get(n))
            .all(|v| v == "SKIP")
        {
            self.event(Event::ChecksumSkipped(source.file_name()))?;
            return Ok(true);
        }

        for (k, sums) in sums {
            if let Some(sum) = sums.get(n) {
                k.verity_file_checksum(self, dirs, source, p, sum, &mut failed)?;
            }
        }

        if !failed.is_empty() {
            self.event(Event::ChecksumFailed(source.file_name(), &failed))?;
            Ok(false)
        } else {
            self.event(Event::ChecksumPass(source.file_name()))?;
            Ok(true)
        }
    }

    pub fn geninteg(&self, options: &Options, p: &Pkgbuild) -> Result<String> {
        use std::fmt::Write;

        let mut arrays = Vec::new();
        let mut output = String::new();
        let dirs = self.pkgbuild_dirs(p)?;

        let mut enabled = p
            .get_all_checksums()
            .into_iter()
            .filter(|(_, v)| !v.is_empty())
            .map(|(k, _)| k)
            .collect::<Vec<_>>();

        if enabled.is_empty() {
            enabled.extend(&self.config.integrity_check);
        }
        if enabled.is_empty() {
            enabled.push(ChecksumKind::Sha512);
        }

        self.download_sources(options, p, true)?;
        self.event(Event::GeneratingChecksums)?;

        for sum in enabled {
            let sums = p.get_checksums(sum);
            match sum {
                ChecksumKind::Md5 => self.gen_integ::<Md5>(&dirs, p, &mut arrays, sums, sum)?,
                ChecksumKind::Sha1 => self.gen_integ::<Sha1>(&dirs, p, &mut arrays, sums, sum)?,
                ChecksumKind::Sha224 => {
                    self.gen_integ::<Sha224>(&dirs, p, &mut arrays, sums, sum)?
                }
                ChecksumKind::Sha256 => {
                    self.gen_integ::<Sha256>(&dirs, p, &mut arrays, sums, sum)?
                }
                ChecksumKind::Sha384 => {
                    self.gen_integ::<Sha384>(&dirs, p, &mut arrays, sums, sum)?
                }
                ChecksumKind::Sha512 => {
                    self.gen_integ::<Sha512>(&dirs, p, &mut arrays, sums, sum)?
                }
                ChecksumKind::Blake2 => {
                    self.gen_integ::<Blake2b512>(&dirs, p, &mut arrays, sums, sum)?
                }
            }
        }

        for (name, mut arr) in arrays {
            let pad = name.len() + 2;
            write!(output, "{}=(", name).unwrap();
            if !arr.is_empty() {
                write!(output, "'{}'", arr.remove(0)).unwrap();
            }
            for val in arr {
                write!(output, "\n{:pad$}'{}'", "", val, pad = pad).unwrap();
            }
            writeln!(output, ")").unwrap();
        }

        let _ = output.pop();

        Ok(output)
    }

    fn gen_integ<D: Digest + Write>(
        &self,
        dirs: &PkgbuildDirs,
        pkgbuild: &Pkgbuild,
        out: &mut Vec<(String, Vec<String>)>,
        sums: &ArchVecs<String>,
        kind: ChecksumKind,
    ) -> Result<()> {
        for arch in &pkgbuild.source.values {
            let default = ArchVec::default();

            let sums = sums.get(arch.arch.as_deref()).unwrap_or(&default);
            let array = self.gen_integ_arr::<D>(dirs, pkgbuild, &arch.values, &sums.values)?;
            let name = match &arch.arch {
                Some(a) => format!("{}_{}", kind, a),
                None => format!("{}", kind),
            };

            out.push((name, array));
        }

        Ok(())
    }

    fn gen_integ_arr<D: Digest + Write>(
        &self,
        dirs: &PkgbuildDirs,
        pkgbuild: &Pkgbuild,
        sources: &[Source],
        sums: &[String],
    ) -> Result<Vec<String>> {
        let mut out = Vec::new();

        for (n, source) in sources.iter().enumerate() {
            if let Some(v) = sums.get(n) {
                if v == "SKIP" {
                    out.push("SKIP".to_string());
                    continue;
                }
            }
            let path = dirs.download_path(source);

            let hash = match source.vcs_kind() {
                Some(vcs) => self.checksum_vcs::<D>(dirs, pkgbuild, vcs, source)?,
                _ => hash_file::<D>(&path)?,
            };
            out.push(hash);
        }

        Ok(out)
    }

    pub(crate) fn verify_file_checksum<D: Digest + Write>(
        &self,
        dirs: &PkgbuildDirs,
        p: &Pkgbuild,
        source: &Source,
        sum: &str,
        name: &'static str,
        failed: &mut Vec<&'static str>,
    ) -> Result<()> {
        let path = dirs.download_path(source);

        if sum == "SKIP" {
            return Ok(());
        }

        let output = match source.vcs_kind() {
            Some(vcs) => self.checksum_vcs::<D>(dirs, p, vcs, source)?,
            _ => hash_file::<D>(&path)?,
        };

        if output != *sum {
            failed.push(name);
        }
        Ok(())
    }
}

fn get_sum_array<'a>(sums: &'a ArchVecs<String>, arch: &Option<String>) -> &'a [String] {
    sums.get(arch.as_deref())
        .map(|v| v.values.as_slice())
        .unwrap_or_default()
}

pub(crate) fn hash_file<D: Digest + Write>(path: &Path) -> Result<String> {
    let mut file = open(File::options().read(true), path, Context::IntegrityCheck)?;
    hash::<D, _>(path, &mut file)
}

pub(crate) fn hash<D: Digest + Write, R: Read>(path: &Path, r: &mut R) -> Result<String> {
    let mut buffer = vec![0; 1024];
    let mut digest = D::new();

    loop {
        let n = match r.read(&mut buffer) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == ErrorKind::Interrupted => continue,
            e => IOErrorExt::context(
                e,
                Context::IntegrityCheck,
                IOContext::HashFile(path.to_path_buf()),
            )?,
        };

        digest.update(&buffer[0..n]);
    }

    Ok(finalize(digest))
}

pub(crate) fn finalize<D: Digest>(digest: D) -> String {
    hex::encode(&digest.finalize())
}
