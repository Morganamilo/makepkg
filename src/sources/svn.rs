use std::process::Command;

use crate::{
    config::PkgbuildDirs,
    error::CommandErrorExt,
    error::{Context, DownloadError, Result},
    fs::{copy_dir, mkdir},
    pkgbuild::{Fragment, Pkgbuild, Source},
    run::CommandOutput,
    sources::VCSKind,
    CommandKind, Event, Makepkg, Options, TOOL_NAME,
};

impl Makepkg {
    pub(crate) fn download_svn(
        &self,
        dirs: &PkgbuildDirs,
        pkgbuild: &Pkgbuild,
        options: &Options,
        source: &Source,
    ) -> Result<()> {
        let repopath = dirs.srcdest.join(source.file_name());
        let mut url = source.url.to_string();
        let mut svnref = "HEAD".to_string();

        if source.protocol() == Some("ssh") {
            url = format!("ssh+{}", url);
        }

        match &source.fragment {
            Some(Fragment::Revision(r)) => svnref = r.to_string(),
            Some(f) => {
                return Err(DownloadError::UnsupportedFragment(
                    source.clone(),
                    VCSKind::Svn,
                    f.clone(),
                )
                .into());
            }
            _ => (),
        }

        if !repopath.exists() {
            self.event(Event::DownloadingVCS(VCSKind::Svn, source.clone()));

            let dir = repopath.join(format!(".{}", TOOL_NAME));
            mkdir(&repopath, Context::RetrieveSources)?;
            mkdir(&dir, Context::RetrieveSources)?;

            let mut command = Command::new("svn");
            command
                .arg("checkout")
                .arg("-r")
                .arg(&svnref)
                .arg("--config-dir")
                .arg(&dir)
                .arg(&url)
                .arg(&repopath)
                .current_dir(&dirs.srcdest)
                .process_spawn(self, CommandKind::DownloadSources(pkgbuild, source))
                .download_context(source, &command, Context::None)?;
        } else if !options.hold_ver {
            self.event(Event::UpdatingVCS(VCSKind::Svn, source.clone()));

            let mut command = Command::new("svn");
            command
                .arg("update")
                .arg("-r")
                .arg(&svnref)
                .current_dir(dirs.download_path(source))
                .process_spawn(self, CommandKind::DownloadSources(pkgbuild, source))
                .download_context(source, &command, Context::None)?;
        }

        Ok(())
    }

    pub(crate) fn extract_svn(&self, dirs: &PkgbuildDirs, source: &Source) -> Result<()> {
        self.event(Event::ExtractingVCS(VCSKind::Svn, source.clone()));

        let repopath = dirs.download_path(source);
        let srcrepopath = dirs.srcdir.join(source.file_name());
        copy_dir(repopath, srcrepopath, Context::ExtractSources)?;
        Ok(())
    }
}
