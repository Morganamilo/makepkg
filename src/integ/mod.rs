use std::fs::File;
use std::io::{ErrorKind, Read};
use std::path::Path;

use blake2::Blake2b512;
use digest::Digest;
use gpgme::{Protocol, SignatureSummary, Validity};
use md5::Md5;
use sha1::Sha1;
use sha2::{Sha224, Sha256, Sha512};

use crate::callback::{Event, LogLevel, LogMessage, SigFailed, SigFailedKind};
use crate::config::PkgbuildDirs;
use crate::error::{
    CommandError, Context, Error, IOContext, IOErrorExt, IntegError, Result, RunErrorKind,
};
use crate::fs::open;
use crate::options::Options;
use crate::pkgbuild::{ArchVec, ArchVecs, Function, Source};
use crate::Makepkg;
//use crate::vcs::is_vcs_proto;
use crate::pkgbuild::Pkgbuild;

impl Makepkg {
    pub fn check_integ(&self, options: &Options, pkgbuild: &Pkgbuild, all: bool) -> Result<()> {
        if options.skip_pgp_check && options.skip_checksums {
            self.log(LogLevel::Warning, LogMessage::SkippingAllIntegrityChecks);
            return Ok(());
        }

        let dirs = self.pkgbuild_dirs(pkgbuild)?;

        if options.skip_checksums {
            self.log(
                LogLevel::Warning,
                LogMessage::SkippingChecksumIntegrityChecks,
            );
            self.check_signatures(pkgbuild, all)?
        } else if options.skip_pgp_check {
            self.log(LogLevel::Warning, LogMessage::SkippingPGPIntegrityChecks);
            self.check_checksums(&dirs, pkgbuild, all)?;
        } else {
            self.check_checksums(&dirs, pkgbuild, all)?;
            self.check_signatures(pkgbuild, all)?;
        }

        if pkgbuild.has_function(Function::Verify) {
            let err = self.run_function(options, pkgbuild, Function::Verify);
            if let Err(Error::Command(CommandError {
                kind: RunErrorKind::ExitCode(_),
                ..
            })) = err
            {
                return Err(IntegError::VerifyFunction.into());
            }
        }

        Ok(())
    }

    pub fn check_signatures(&self, pkgbuild: &Pkgbuild, all: bool) -> Result<()> {
        self.event(Event::VerifyingSignatures);
        let mut gpg =
            gpgme::Context::from_protocol(Protocol::OpenPgp).map_err(|e| IntegError::Gpgme(e))?;
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

    pub fn check_sigs_one_arch(
        &self,
        dirs: &PkgbuildDirs,
        gpg: &mut gpgme::Context,
        pkgbuild: &Pkgbuild,
        sources: &ArchVec<Source>,
    ) -> Result<bool> {
        let mut ok = true;

        for source in &sources.values {
            /*match source.protocol() {
                Some(proto) if is_vcs_proto(proto) => {
                    ok &= self.verify_vcs_sig(gpg, pkgbuild, source)?;
                    continue;
                }
                _ => (),
            }*/

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

            let sig = self.download_path(dirs, &source);
            let data = self.download_path(dirs, source_file);
            let sig = open(File::options().read(true), sig, Context::IntegrityCheck)?;
            let data = open(File::options().read(true), data, Context::IntegrityCheck)?;

            let res = gpg
                .verify_detached(sig, data)
                .map_err(|e| IntegError::Gpgme(e))?;
            ok &= self.process_sig(source_file, pkgbuild, &res)?;
        }

        Ok(ok)
    }

    pub fn process_sig(
        &self,
        source: &Source,
        pkgbuild: &Pkgbuild,
        res: &gpgme::VerificationResult,
    ) -> Result<bool> {
        let mut ok = true;

        let file = source.file_name();
        self.event(Event::VerifyingSignature(file.to_string()));

        for sig in res.signatures() {
            let fingerprint = sig
                .fingerprint()
                .map_err(|_| IntegError::ReadFingerprint(file.to_string()))?;
            if let Err(err) = sig.status() {
                ok = false;

                if sig.summary().contains(SignatureSummary::KEY_MISSING) {
                    self.event(
                        SigFailed::new(file, fingerprint, SigFailedKind::UnknownPublicKey).into(),
                    );
                } else if sig.summary().contains(SignatureSummary::KEY_REVOKED) {
                    self.event(SigFailed::new(file, fingerprint, SigFailedKind::Revoked).into());
                } else if sig.summary().contains(SignatureSummary::KEY_REVOKED) {
                    self.event(SigFailed::new(file, fingerprint, SigFailedKind::Expired).into());
                } else {
                    let d = err.to_string();
                    self.event(
                        SigFailed::new(file, fingerprint, SigFailedKind::Other(d).into()).into(),
                    );
                }
                continue;
            }

            if pkgbuild.validpgpkeys.is_empty() {
                if !matches!(
                    sig.validity(),
                    Validity::Full | Validity::Marginal | Validity::Ultimate
                ) {
                    self.event(SigFailed::new(file, fingerprint, SigFailedKind::NotTrusted).into());
                    ok = false;
                }
            } else if !pkgbuild.validpgpkeys.iter().any(|p| p == fingerprint) {
                self.event(
                    SigFailed::new(file, fingerprint, SigFailedKind::NotInValidPgpKeys).into(),
                );
                ok = false;
            } else {
                self.event(Event::SignatureCheckPass(file.to_string()))
            }
        }

        Ok(ok)
    }

    fn check_checksums(&self, dirs: &PkgbuildDirs, pkgbuild: &Pkgbuild, all: bool) -> Result<()> {
        self.event(Event::VerifyingChecksums);

        let mut ok = true;

        for source in &pkgbuild.source.values {
            if !all && !source.enabled(&self.config.arch) {
                continue;
            }
            let md5 = get_sum_array(&pkgbuild.md5sums, &source.arch);
            let sha1 = get_sum_array(&pkgbuild.sha1sums, &source.arch);
            let sha224 = get_sum_array(&pkgbuild.sha224sums, &source.arch);
            let sha256 = get_sum_array(&pkgbuild.sha256sums, &source.arch);
            let sha512 = get_sum_array(&pkgbuild.sha512sums, &source.arch);
            let b2 = get_sum_array(&pkgbuild.b2sums, &source.arch);

            for (n, source) in source.values.iter().enumerate() {
                ok &= self.check_checksums_one_file(
                    dirs, source, n, md5, &sha1, &sha224, &sha256, &sha512, &b2,
                )?;
            }
        }

        if !ok {
            return Err(IntegError::VerifyFunction.into());
        }

        Ok(())
    }

    fn check_checksums_one_file(
        &self,
        dirs: &PkgbuildDirs,
        source: &Source,
        n: usize,
        md5: &[String],
        sha1: &[String],
        sha224: &[String],
        sha256: &[String],
        sha512: &[String],
        b2: &[String],
    ) -> Result<bool> {
        let mut failed = Vec::new();
        self.event(Event::VerifyingChecksum(source.file_name().to_string()));

        if [
            md5.get(n),
            sha1.get(n),
            sha224.get(n),
            sha256.get(n),
            sha512.get(n),
            b2.get(n),
        ]
        .iter()
        .flatten()
        .all(|v| *v == "SKIP")
        {
            self.event(Event::ChecksumSkipped(source.file_name().to_string()));
            return Ok(true);
        }

        self.verify_file_checksum::<Md5>(dirs, source, md5.get(n), "MD5", &mut failed)?;
        self.verify_file_checksum::<Sha1>(dirs, source, sha1.get(n), "SHA1", &mut failed)?;
        self.verify_file_checksum::<Sha224>(dirs, source, sha224.get(n), "SHA224", &mut failed)?;
        self.verify_file_checksum::<Sha256>(dirs, source, sha256.get(n), "SHA256", &mut failed)?;
        self.verify_file_checksum::<Sha512>(dirs, source, sha512.get(n), "SHA512", &mut failed)?;
        self.verify_file_checksum::<Blake2b512>(dirs, source, b2.get(n), "B2", &mut failed)?;

        if !failed.is_empty() {
            self.event(Event::ChecksumFailed(
                source.file_name().to_string(),
                failed.into_iter().map(|s| s.to_string()).collect(),
            ));
            Ok(false)
        } else {
            self.event(Event::ChecksumPass(source.file_name().to_string()));
            Ok(true)
        }
    }

    pub fn geninteg(&self, options: &Options, p: &Pkgbuild) -> Result<String> {
        use std::fmt::Write;

        let mut enabled = Vec::new();
        let mut arrays = Vec::new();
        let mut output = String::new();
        let dirs = self.pkgbuild_dirs(p)?;

        if !p.md5sums.is_empty() {
            enabled.push("md5");
        }
        if !p.sha224sums.is_empty() {
            enabled.push("sha224");
        }
        if !p.sha256sums.is_empty() {
            enabled.push("sha256");
        }
        if !p.sha512sums.is_empty() {
            enabled.push("sha512");
        }
        if !p.b2sums.is_empty() {
            enabled.push("b2");
        }
        if enabled.is_empty() {
            enabled.extend(self.config.integrity_check.iter().map(|s| s.as_str()));
        }
        if enabled.is_empty() {
            enabled.push("sha512")
        }

        self.download_sources(options, p, true)?;

        self.event(Event::GeneratingChecksums);

        for sum in enabled {
            match sum {
                "md5" => self.gen_integ::<Md5>(&dirs, &mut arrays, &p.source, &p.md5sums, sum)?,
                "sha1" => {
                    self.gen_integ::<Sha1>(&dirs, &mut arrays, &p.source, &p.sha1sums, sum)?
                }
                "sha224" => {
                    self.gen_integ::<Sha224>(&dirs, &mut arrays, &p.source, &p.sha224sums, sum)?
                }
                "sha256" => {
                    self.gen_integ::<Sha256>(&dirs, &mut arrays, &p.source, &p.sha256sums, sum)?
                }
                "sha512" => {
                    self.gen_integ::<Sha512>(&dirs, &mut arrays, &p.source, &p.sha512sums, sum)?
                }
                "b2" => {
                    self.gen_integ::<Blake2b512>(&dirs, &mut arrays, &p.source, &p.b2sums, sum)?
                }
                _ => (),
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

        Ok(output)
    }

    fn gen_integ<D: Digest>(
        &self,
        dirs: &PkgbuildDirs,
        out: &mut Vec<(String, Vec<String>)>,
        sources: &ArchVecs<Source>,
        sums: &ArchVecs<String>,
        sum: &str,
    ) -> Result<()> {
        for arch in &sources.values {
            let default = ArchVec::default();

            let sums = sums.get(arch.arch.as_deref()).unwrap_or(&default);
            let array = self.gen_integ_arr::<D>(dirs, &arch.values, &sums.values)?;
            let name = match &arch.arch {
                Some(a) => format!("{}sums_{}", sum, a),
                None => format!("{}sums", sum),
            };

            out.push((name, array));
        }

        Ok(())
    }

    fn gen_integ_arr<D: Digest>(
        &self,
        dirs: &PkgbuildDirs,
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
            let path = self.download_path(dirs, source);

            let hash = match source.protocol() {
                //Some(proto) if is_vcs_proto(proto) => self.checksum_vcs::<D>(source)?,
                _ => hash_file::<D>(&path)?,
            };
            out.push(hash);
        }

        Ok(out)
    }

    fn verify_file_checksum<D: Digest>(
        &self,
        dirs: &PkgbuildDirs,
        source: &Source,
        sum: Option<&String>,
        name: &'static str,
        failed: &mut Vec<&'static str>,
    ) -> Result<()> {
        let path = self.download_path(dirs, source);

        let sum = if let Some(sum) = sum {
            sum
        } else {
            return Ok(());
        };

        if sum == "SKIP" {
            return Ok(());
        }

        let output = match source.protocol() {
            //Some(proto) if is_vcs_proto(proto) => self.checksum_vcs::<D>(source)?,
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

pub fn hash_file<D: Digest>(path: &Path) -> Result<String> {
    let mut file = open(File::options().read(true), path, Context::IntegrityCheck)?;
    hash::<D, _>(path, &mut file)
}

pub fn hash<D: Digest, R: Read>(path: &Path, r: &mut R) -> Result<String> {
    let mut buffer = vec![0; 1024];
    let mut digest = D::new();

    loop {
        let n = match r.read(&mut buffer) {
            Ok(n) if n == 0 => break,
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

    let output = digest.finalize();
    let output = hex::encode(&output);
    Ok(output)
}
