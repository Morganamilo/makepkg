use std::process::{Command, Stdio};

use digest::Digest;

use crate::{
    config::PkgbuildDirs,
    error::{CommandError, CommandErrorExt, Context, DownloadError, Result},
    integ,
    pkgbuild::{Fragment, Source},
    sources::VCSKind,
    Event, Makepkg, Options,
};

impl Makepkg {
    pub(crate) fn download_hg(
        &self,
        dirs: &PkgbuildDirs,
        options: &Options,
        source: &Source,
    ) -> Result<()> {
        let repopath = dirs.download_path(source);
        let mut url = source.url.to_string();

        if source.protocol().as_deref() == Some("ssh") {
            url = format!("ssh+{}", url);
        }

        if !repopath.exists() {
            self.event(Event::DownloadingVCS(VCSKind::Mercurial, source.clone()));

            let mut command = Command::new("hg");
            command
                .arg("clone")
                .arg("-U")
                .arg(&url)
                .arg(&repopath)
                .current_dir(&dirs.srcdest)
                .status()
                .download_context(source, &command, Context::None)?;
        } else if !options.hold_ver {
            self.event(Event::UpdatingVCS(VCSKind::Mercurial, source.clone()));

            let mut command = Command::new("hg");
            command
                .arg("pull")
                .current_dir(repopath)
                .status()
                .download_context(source, &command, Context::None)?;
        }

        Ok(())
    }

    pub(crate) fn extract_hg(&self, dirs: &PkgbuildDirs, source: &Source) -> Result<()> {
        self.event(Event::ExtractingVCS(VCSKind::Mercurial, source.clone()));

        let srcpath = dirs.srcdir.join(source.file_name());
        let repopath = dirs.download_path(source);
        let mut hgref = "default".to_string();

        let mut command = Command::new("hg");
        if command
            .arg("identify")
            .arg("-r")
            .arg("@")
            .arg(&repopath)
            .current_dir(&dirs.srcdest)
            .status()
            .map_err(|e| {
                DownloadError::Command(
                    source.clone(),
                    CommandError::exec(e, &command, Context::ExtractSources),
                )
            })?
            .success()
        {
            hgref = "@".to_string();
        }

        match &source.fragment {
            Some(Fragment::Branch(r) | Fragment::Revision(r) | Fragment::Tag(r)) => {
                hgref = r.to_string()
            }
            Some(f) => {
                return Err(DownloadError::UnsupportedFragment(
                    source.clone(),
                    VCSKind::Mercurial,
                    f.clone(),
                )
                .into());
            }
            _ => (),
        }

        if srcpath.exists() {
            let mut command = Command::new("hg");
            command
                .arg("pull")
                .current_dir(&srcpath)
                .status()
                .download_context(source, &command, Context::None)?;
            command = Command::new("hg");
            command
                .arg("update")
                .arg("-Cr")
                .arg(&hgref)
                .current_dir(&srcpath)
                .status()
                .download_context(source, &command, Context::None)?;
        } else {
            let mut command = Command::new("hg");
            command
                .arg("clone")
                .arg("-u")
                .arg(&hgref)
                .arg(&repopath)
                .arg(&srcpath)
                .status()
                .download_context(source, &command, Context::None)?;
        }

        Ok(())
    }

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
            Some(f) => {
                return Err(DownloadError::UnsupportedFragment(
                    source.clone(),
                    VCSKind::Mercurial,
                    f.clone(),
                )
                .into());
            }
            None => Ok("SKIP".to_string()),
        }
    }
}
