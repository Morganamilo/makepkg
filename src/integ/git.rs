use std::process::{Command, Stdio};

use digest::Digest;

use crate::{
    config::PkgbuildDirs,
    error::{CommandErrorExt, CommandOutputExt, Context, DownloadError, IntegError, Result},
    integ,
    pkgbuild::{Fragment, Pkgbuild, Source},
    sources::VCSKind,
    Event, Makepkg, SigFailed, SigFailedKind,
};

impl Makepkg {
    pub fn checksum_git<D: Digest>(&self, dirs: &PkgbuildDirs, source: &Source) -> Result<String> {
        let srcpath = dirs.download_path(source);

        match &source.fragment {
            Some(Fragment::Tag(r) | Fragment::Commit(r)) => {
                let mut command = Command::new("git");
                let mut child = command
                    .arg("-c")
                    .arg("core.abbrev=no")
                    .arg("archive")
                    .arg("--format")
                    .arg("tar")
                    .arg(r)
                    .stdout(Stdio::piped())
                    .current_dir(&srcpath)
                    .spawn()
                    .cmd_context(&command, Context::IntegrityCheck)?;

                let mut stdout = child.stdout.take().unwrap();
                let hash = integ::hash::<D, _>(source.file_name().as_ref(), &mut stdout)?;

                child
                    .wait()
                    .cmd_context(&command, Context::IntegrityCheck)?;

                Ok(hash)
            }
            Some(f) => {
                Err(
                    DownloadError::UnsupportedFragment(source.clone(), VCSKind::Git, f.clone())
                        .into(),
                )
            }

            None => Ok("SKIP".to_string()),
        }
    }

    pub(crate) fn verify_git_sig(
        &self,
        dirs: &PkgbuildDirs,
        pkgbuild: &Pkgbuild,
        source: &Source,
        gpg: &mut gpgme::Context,
    ) -> Result<bool> {
        let path = dirs.download_path(source);
        let fragval = match &source.fragment {
            Some(Fragment::Tag(r) | Fragment::Commit(r) | Fragment::Branch(r)) => r.as_str(),
            _ => "HEAD",
        };

        let mut command = Command::new("git");
        let object = command
            .arg("cat-file")
            .arg("-p")
            .arg(fragval)
            .current_dir(path)
            .output()
            .read(&command, Context::IntegrityCheck)
            .map_err(|_| IntegError::SignatureNotFound(source.clone()))?;

        if !object.contains("-----BEGIN PGP SIGNATURE-----") {
            self.event(Event::SignatureCheckFailed(SigFailed::new(
                source.file_name(),
                "none",
                SigFailedKind::NotSigned,
            )));
            return Ok(false);
        }

        let sig = object.replace("\ngpgsig ", "\n");

        let mut keep = true;
        let mut object = object
            .lines()
            .filter(|line| {
                if line.contains("-----BEGIN PGP SIGNATURE-----") {
                    keep = false;
                    keep
                } else if line.contains("-----END PGP SIGNATURE-----") {
                    keep = true;
                    false
                } else {
                    keep
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        object.push('\n');

        let res = gpg
            .verify_detached(sig, object)
            .map_err(IntegError::Gpgme)?;
        self.process_sig(source, pkgbuild, &res)
    }
}
