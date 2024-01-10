use std::process::{Command, Stdio};

use digest::Digest;

use crate::{
    config::PkgbuildDirs,
    error::{CommandErrorExt, Context, DownloadError, Result},
    integ,
    pkgbuild::{Fragment, Source},
    sources::VCSKind,
    Makepkg,
};

impl Makepkg {
    pub fn checksum_bzr<D: Digest>(&self, dirs: &PkgbuildDirs, source: &Source) -> Result<String> {
        let srcpath = dirs.download_path(source);

        match &source.fragment {
            Some(Fragment::Revision(r)) => {
                let mut command = Command::new("bzr");
                let mut child = command
                    .arg("export")
                    .arg("--directory")
                    .arg(&srcpath)
                    .arg("--format")
                    .arg("tar")
                    .arg("--revision")
                    .arg(r)
                    .arg("-")
                    .stdout(Stdio::piped())
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
                    DownloadError::UnsupportedFragment(source.clone(), VCSKind::Bzr, f.clone())
                        .into(),
                )
            }
            None => Ok("SKIP".to_string()),
        }
    }
}
