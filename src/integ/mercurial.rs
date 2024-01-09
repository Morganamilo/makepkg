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
    pub(crate) fn checksum_hg<D: Digest>(
        &self,
        dirs: &PkgbuildDirs,
        source: &Source,
    ) -> Result<String> {
        let srcpath = dirs.download_path(source);

        match &source.fragment {
            Some(Fragment::Tag(r) | Fragment::Revision(r)) => {
                let mut command = Command::new("hg");
                let mut child = command
                    .arg("--repository")
                    .arg(&srcpath)
                    .arg("archive")
                    .arg("--type")
                    .arg("tar")
                    .arg("--rev")
                    .arg(r)
                    .arg("-")
                    .stdout(Stdio::piped())
                    .spawn()
                    .download_context(source, &command, Context::None)?;

                let mut stdout = child.stdout.take().unwrap();
                let hash = integ::hash::<D, _>(source.file_name().as_ref(), &mut stdout)?;

                child
                    .wait()
                    .download_context(source, &command, Context::None)?;

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
