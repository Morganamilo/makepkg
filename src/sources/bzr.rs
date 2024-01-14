use std::process::Command;

use crate::{
    config::PkgbuildDirs,
    error::{CommandErrorExt, Context, DownloadError, Result},
    pkgbuild::{Fragment, Pkgbuild, Source},
    run::CommandOutput,
    sources::VCSKind,
    CommandKind, Event, Makepkg, Options,
};

impl Makepkg {
    pub(crate) fn download_bzr(
        &self,
        dirs: &PkgbuildDirs,
        pkgbuild: &Pkgbuild,
        options: &Options,
        source: &Source,
    ) -> Result<()> {
        let repopath = dirs.srcdest.join(source.file_name());
        let mut url = source.url.to_string();

        if source.protocol() == Some("ssh") {
            url = format!("bzr+{}", url);
        }

        if !repopath.exists() {
            self.event(Event::DownloadingVCS(VCSKind::Bzr, source))?;

            let mut command = Command::new("bzr");
            command
                .arg("branch")
                .arg(&url)
                .arg(&repopath)
                .arg("--no-tree")
                .arg("--use-existing-dir")
                .process_spawn(self, CommandKind::DownloadSources(pkgbuild, source))
                .download_context(source, &command, Context::None)?;
        } else if !options.hold_ver {
            self.event(Event::UpdatingVCS(VCSKind::Bzr, source))?;

            let mut command = Command::new("bzr");
            command
                .arg("pull")
                .arg(&url)
                .current_dir(&repopath)
                .process_spawn(self, CommandKind::DownloadSources(pkgbuild, source))
                .download_context(source, &command, Context::None)?;
        }

        Ok(())
    }

    pub fn extract_bzr(
        &self,
        dirs: &PkgbuildDirs,
        pkgbuild: &Pkgbuild,
        source: &Source,
    ) -> Result<()> {
        self.event(Event::ExtractingVCS(VCSKind::Bzr, source))?;

        let srcpath = dirs.srcdir.join(source.file_name());
        let repopath = dirs.download_path(source);
        let mut bzrref = "last:1".to_string();

        match &source.fragment {
            Some(Fragment::Revision(r)) => bzrref = r.to_string(),
            Some(f) => {
                return Err(DownloadError::UnsupportedFragment(
                    source.clone(),
                    VCSKind::Bzr,
                    f.clone(),
                )
                .into());
            }
            _ => (),
        }

        if srcpath.exists() {
            let mut command = Command::new("bzr");
            command
                .arg("pull")
                .arg(&repopath)
                .arg("-q")
                .arg("--overwrite")
                .arg("-r")
                .arg(&bzrref)
                .current_dir(&srcpath)
                .process_spawn(self, CommandKind::DownloadSources(pkgbuild, source))
                .download_context(source, &command, Context::None)?;
            command = Command::new("bzr");
            command
                .arg("clean-tree")
                .arg("-q")
                .arg("--detritus")
                .arg("--force")
                .current_dir(&srcpath)
                .process_spawn(self, CommandKind::DownloadSources(pkgbuild, source))
                .download_context(source, &command, Context::None)?;
        } else {
            let mut command = Command::new("bzr");
            command
                .arg("checkout")
                .arg(&repopath)
                .arg("-r")
                .arg(&bzrref)
                .current_dir(&dirs.srcdir)
                .process_spawn(self, CommandKind::DownloadSources(pkgbuild, source))
                .download_context(source, &command, Context::None)?;
        }

        Ok(())
    }
}
