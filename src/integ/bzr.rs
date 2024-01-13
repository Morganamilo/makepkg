use std::{io::Write, process::Command};

use digest::Digest;

use crate::{
    config::PkgbuildDirs,
    error::{CommandErrorExt, Context, DownloadError, Result},
    pkgbuild::{Fragment, Pkgbuild, Source},
    run::CommandOutput,
    sources::VCSKind,
    CommandKind, Makepkg,
};

use super::finalize;

impl Makepkg {
    pub fn checksum_bzr<D: Digest + Write>(
        &self,
        dirs: &PkgbuildDirs,
        pkgbuild: &Pkgbuild,
        source: &Source,
    ) -> Result<String> {
        let srcpath = dirs.download_path(source);

        match &source.fragment {
            Some(Fragment::Revision(r)) => {
                let mut digest = D::new();

                let mut command = Command::new("bzr");
                command
                    .arg("export")
                    .arg("--directory")
                    .arg(&srcpath)
                    .arg("--format")
                    .arg("tar")
                    .arg("--revision")
                    .arg(r)
                    .arg("-")
                    .process_write_output(
                        self,
                        CommandKind::DownloadSources(pkgbuild, source),
                        &mut digest,
                    )
                    .cmd_context(&command, Context::IntegrityCheck)?;

                let hash = finalize(digest);
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
