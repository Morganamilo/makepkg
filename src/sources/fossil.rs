use std::path::Path;
use std::process::Command;

use crate::{
    config::PkgbuildDirs,
    error::{CommandErrorExt, CommandOutputExt, Context, DownloadError, Result},
    pkgbuild::{Fragment, Source},
    sources::VCSKind,
    Event, Makepkg, Options,
};

impl Makepkg {
    pub(crate) fn download_fossil(
        &self,
        dirs: &PkgbuildDirs,
        options: &Options,
        source: &Source,
    ) -> Result<()> {
        let repopath = dirs.download_path(source);
        if !repopath.exists() {
            self.event(Event::DownloadingVCS(VCSKind::Fossil, source.clone()));

            let mut command = Command::new("fossil");
            command
                .arg("clone")
                .arg(&source.url)
                .arg(&repopath)
                .status()
                .download_context(source, &command, Context::None)?;
        } else if !options.hold_ver {
            self.event(Event::UpdatingVCS(VCSKind::Fossil, source.clone()));

            let mut command = Command::new("fossil");
            let url = command
                .arg("remote")
                .arg("-R")
                .arg(&repopath)
                .output()
                .download_read(source, &command, Context::None)?;

            if url != source.url {
                return Err(DownloadError::RemotesDiffer(source.clone(), url.trim().into()).into());
            }

            let mut command = Command::new("fossil");
            command
                .arg("pull")
                .arg("-R")
                .arg(&repopath)
                .status()
                .download_context(source, &command, Context::None)?;
        }

        Ok(())
    }

    pub(crate) fn extract_fossil(&self, dirs: &PkgbuildDirs, source: &Source) -> Result<()> {
        self.event(Event::ExtractingVCS(VCSKind::Fossil, source.clone()));

        let srcpath = dirs.srcdir.join(source.file_name());
        let repopath = dirs.download_path(source);
        let mut fref = "tip".to_string();

        if srcpath.exists() {
            if srcpath.join(".fslckout").exists() {
                let mut command = Command::new("fossil");

                let info = command
                    .arg("info")
                    .current_dir(&srcpath)
                    .output()
                    .download_read(source, &command, Context::None)?;

                let repository = info
                    .trim()
                    .lines()
                    .find(|l| l.starts_with("repository:"))
                    .map(|l| {
                        l.splitn(2, |c| char::is_whitespace(c))
                            .last()
                            .unwrap()
                            .trim_start()
                    })
                    .unwrap_or_default();

                if Path::new(repository) != repopath.as_path() {
                    return Err(
                        DownloadError::RemotesDiffer(source.clone(), repository.into()).into(),
                    );
                }
            } else {
                return Err(DownloadError::NotCheckedOut(source.clone()).into());
            }
        } else {
            let mut command = Command::new("fossil");
            command
                .arg("open")
                .arg(&repopath)
                .arg("--workdir")
                .arg(&dirs.srcdir)
                .current_dir(&dirs.srcdir)
                .status()
                .download_context(source, &command, Context::None)?;
        }

        match &source.fragment {
            Some(Fragment::Branch(r) | Fragment::Commit(r) | Fragment::Tag(r)) => {
                fref = r.to_string()
            }
            Some(f) => {
                return Err(DownloadError::UnsupportedFragment(
                    source.clone(),
                    VCSKind::Fossil,
                    f.clone(),
                )
                .into());
            }
            _ => (),
        }

        let mut command = Command::new("fossil");
        command
            .arg("update")
            .arg(&fref)
            .current_dir(&srcpath)
            .status()
            .download_context(source, &command, Context::None)?;

        Ok(())
    }
}
