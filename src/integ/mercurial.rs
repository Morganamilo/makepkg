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
    pub(crate) fn checksum_hg<D: Digest + Write>(
        &self,
        dirs: &PkgbuildDirs,
        pkgbuild: &Pkgbuild,
        source: &Source,
    ) -> Result<String> {
        let srcpath = dirs.download_path(source);

        match &source.fragment {
            Some(Fragment::Tag(r) | Fragment::Revision(r)) => {
                let mut digest = D::new();

                let mut command = Command::new("hg");
                command
                    .arg("--repository")
                    .arg(&srcpath)
                    .arg("archive")
                    .arg("--type")
                    .arg("tar")
                    .arg("--rev")
                    .arg(r)
                    .arg("-")
                    .process_write_output(self, CommandKind::Integ(pkgbuild, source), &mut digest)
                    .download_context(source, &command, Context::None)?;

                let hash = finalize(digest);
                Ok(hash)
            }
            Some(f) => Err(DownloadError::UnsupportedFragment(
                source.clone(),
                VCSKind::Mercurial,
                f.clone(),
            )
            .into()),
            None => Ok("SKIP".to_string()),
        }
    }
}
