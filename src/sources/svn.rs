use std::process::Command;

use walkdir::WalkDir;

use crate::{
    config::PkgbuildDirs,
    error::{CommandErrorExt, IOContext, IOErrorExt},
    error::{Context, DownloadError, Result},
    fs::{copy, make_link, mkdir, read_link},
    pkgbuild::{Fragment, Source},
    sources::VCSKind,
    Event, Makepkg, Options, TOOL_NAME,
};

impl Makepkg {
    pub(crate) fn download_svn(
        &self,
        dirs: &PkgbuildDirs,
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
                .status()
                .download_context(source, &command, Context::None)?;
        } else if !options.hold_ver {
            self.event(Event::UpdatingVCS(VCSKind::Svn, source.clone()));

            let mut command = Command::new("svn");
            command
                .arg("update")
                .arg("-r")
                .arg(&svnref)
                .current_dir(dirs.download_path(source))
                .status()
                .download_context(source, &command, Context::None)?;
        }

        Ok(())
    }

    pub(crate) fn extract_svn(&self, dirs: &PkgbuildDirs, source: &Source) -> Result<()> {
        self.event(Event::ExtractingVCS(VCSKind::Svn, source.clone()));

        let repopath = dirs.download_path(source);
        let srcrepopath = dirs.srcdir.join(source.file_name());
        mkdir(srcrepopath, Context::ExtractSources)?;

        // Walk through all files / dirs in the cloned repo as we cannot directly `cp -r`, instead copy individually
        for file in WalkDir::new(&repopath) {
            let file = file.context(
                Context::ExtractSources,
                IOContext::ReadDir(repopath.clone()),
            )?;
            let ty = file.file_type();
            let rel_path = &file.path().strip_prefix(&dirs.srcdest).context(
                Context::ExtractSources,
                IOContext::ReadDir(repopath.clone()),
            )?;
            let path = dirs.srcdir.join(rel_path);

            if ty.is_dir() {
                mkdir(&path, Context::ExtractSources)?;
            } else if ty.is_symlink() {
                let pointer = read_link(file.path(), Context::ExtractSources)?;
                make_link(pointer, path, Context::ExtractSources)?;
            } else {
                copy(file.path(), path, Context::ExtractSources)?;
            }
        }

        Ok(())
    }
}
