use std::process::Command;

use crate::{
    config::PkgbuildDirs,
    error::{CommandError, CommandErrorExt, Context, DownloadError, Result},
    pkgbuild::{Fragment, Pkgbuild, Source},
    run::CommandOutput,
    sources::VCSKind,
    CommandKind, Event, Makepkg, Options,
};

impl Makepkg {
    pub(crate) fn download_hg(
        &self,
        dirs: &PkgbuildDirs,
        pkgbuild: &Pkgbuild,
        options: &Options,
        source: &Source,
    ) -> Result<()> {
        let repopath = dirs.download_path(source);
        let mut url = source.url.to_string();

        if source.protocol() == Some("ssh") {
            url = format!("ssh+{}", url);
        }

        if !repopath.exists() {
            self.event(Event::DownloadingVCS(VCSKind::Mercurial, source.clone()))?;

            let mut command = Command::new("hg");
            command
                .arg("clone")
                .arg("-U")
                .arg(&url)
                .arg(&repopath)
                .current_dir(&dirs.srcdest)
                .process_spawn(self, CommandKind::DownloadSources(pkgbuild, source))
                .download_context(source, &command, Context::None)?;
        } else if !options.hold_ver {
            self.event(Event::UpdatingVCS(VCSKind::Mercurial, source.clone()))?;

            let mut command = Command::new("hg");
            command
                .arg("pull")
                .current_dir(repopath)
                .process_spawn(self, CommandKind::DownloadSources(pkgbuild, source))
                .download_context(source, &command, Context::None)?;
        }

        Ok(())
    }

    pub(crate) fn extract_hg(
        &self,
        dirs: &PkgbuildDirs,
        pkgbuild: &Pkgbuild,
        source: &Source,
    ) -> Result<()> {
        self.event(Event::ExtractingVCS(VCSKind::Mercurial, source.clone()))?;

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
            .process_output()
            .map_err(|e| {
                DownloadError::Command(
                    source.clone(),
                    CommandError::exec(e, &command, Context::ExtractSources),
                )
            })?
            .status
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
                .process_spawn(self, CommandKind::ExtractSources(pkgbuild, source))
                .download_context(source, &command, Context::None)?;
            command = Command::new("hg");
            command
                .arg("update")
                .arg("-Cr")
                .arg(&hgref)
                .current_dir(&srcpath)
                .process_spawn(self, CommandKind::ExtractSources(pkgbuild, source))
                .download_context(source, &command, Context::None)?;
        } else {
            let mut command = Command::new("hg");
            command
                .arg("clone")
                .arg("-u")
                .arg(&hgref)
                .arg(&repopath)
                .arg(&srcpath)
                .process_spawn(self, CommandKind::ExtractSources(pkgbuild, source))
                .download_context(source, &command, Context::None)?;
        }

        Ok(())
    }
}
