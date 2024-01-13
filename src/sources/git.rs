use std::process::Command;

use crate::{
    callback::{CommandKind, Event},
    config::PkgbuildDirs,
    error::{CommandErrorExt, CommandOutputExt, Context, DownloadError, Result},
    pkgbuild::{Fragment, Pkgbuild, Source},
    run::CommandOutput,
    sources::VCSKind,
    Makepkg, Options, TOOL_NAME,
};

impl Makepkg {
    pub(crate) fn download_git(
        &self,
        dirs: &PkgbuildDirs,
        pkgbuild: &Pkgbuild,
        options: &Options,
        source: &Source,
    ) -> Result<()> {
        let path = dirs.download_path(source);

        if !path.exists() || !path.join("objects").exists() {
            self.event(Event::DownloadingVCS(VCSKind::Git, source.clone()));

            let flags = std::env::var("GITFLAGS");
            let flags = flags
                .as_ref()
                .map(|v| v.split_whitespace().collect::<Vec<_>>());
            let flags = flags.as_deref().unwrap_or(["--mirror"].as_slice());

            let mut command = Command::new("git");
            command
                .arg("clone")
                .arg("--origin=origin")
                .args(flags)
                .arg("--")
                .arg(&source.url)
                .arg(path)
                .env("GIT_TERMINAL_PROMPT", "0")
                .process_spawn(self, CommandKind::DownloadSources(pkgbuild, source))
                .download_context(source, &command, Context::None)?;
        } else if !options.hold_ver {
            let mut command = Command::new("git");
            let remote_url = command
                .arg("config")
                .arg("--get")
                .arg("remote.origin.url")
                .current_dir(dirs.download_path(source))
                .process_read(self, CommandKind::DownloadSources(pkgbuild, source))
                .download_read(source, &command, Context::None)?;

            if remote_url.trim_end_matches(".git") != source.url.trim_end_matches(".git") {
                return Err(
                    DownloadError::RemotesDiffer(source.clone(), remote_url.clone()).into(),
                );
            }

            self.event(Event::UpdatingVCS(VCSKind::Git, source.clone()));

            let mut command = Command::new("git");
            command
                .arg("fetch")
                .arg("--all")
                .arg("-p")
                .env("GIT_TERMINAL_PROMPT", "0")
                .current_dir(dirs.download_path(source))
                .process_spawn(self, CommandKind::DownloadSources(pkgbuild, source))
                .download_context(source, &command, Context::None)?;
        }

        Ok(())
    }

    pub(crate) fn extract_git(
        &self,
        dirs: &PkgbuildDirs,
        pkgbuild: &Pkgbuild,
        source: &Source,
    ) -> Result<()> {
        let mut gitref = "origin/HEAD".to_string();
        let mut updating = false;
        let srcpath = dirs.srcdir.join(source.file_name());
        self.event(Event::ExtractingVCS(VCSKind::Git, source.clone()));

        if srcpath.exists() {
            updating = true;
            let mut command = Command::new("git");
            command
                .arg("fetch")
                .current_dir(&srcpath)
                .process_spawn(self, CommandKind::ExtractSources(pkgbuild, source))
                .download_context(source, &command, Context::None)?;
        } else {
            let mut command = Command::new("git");
            command
                .arg("clone")
                .arg("--origin=origin")
                .arg("-s")
                .arg(dirs.srcdest.join(source.file_name()))
                .arg(source.file_name())
                .current_dir(&dirs.srcdir)
                .env("GIT_TERMINAL_PROMPT", "0")
                .process_spawn(self, CommandKind::ExtractSources(pkgbuild, source))
                .download_context(source, &command, Context::None)?;
        }

        match &source.fragment {
            Some(Fragment::Commit(r) | Fragment::Tag(r)) => gitref = r.to_string(),
            Some(Fragment::Branch(r)) => gitref = format!("origin/{}", r),
            Some(f) => {
                return Err(DownloadError::UnsupportedFragment(
                    source.clone(),
                    VCSKind::Git,
                    f.clone(),
                )
                .into());
            }
            _ => (),
        }

        if let Some(frag @ Fragment::Tag(_)) = &source.fragment {
            let mut command = Command::new("git");
            let tagname = command
                .arg("tag")
                .arg("-l")
                .arg("--format=%(tag)")
                .arg(&gitref)
                .current_dir(&srcpath)
                .process_read(self, CommandKind::DownloadSources(pkgbuild, source))
                .download_read(source, &command, Context::None)?;

            if tagname.is_empty() {
                return Err(DownloadError::RefNotFound(source.clone(), frag.clone()).into());
            }

            if tagname != gitref {
                return Err(DownloadError::RefsDiffer(
                    source.clone(),
                    gitref.clone(),
                    tagname.clone(),
                )
                .into());
            }
        }

        if gitref != "origin/head" || updating {
            let mut command = Command::new("git");
            command
                .arg("checkout")
                .arg("--force")
                .arg("--no-track")
                .arg("-B")
                .arg(TOOL_NAME)
                .arg(&gitref)
                .arg("--")
                .current_dir(&srcpath)
                .process_spawn(self, CommandKind::ExtractSources(pkgbuild, source))
                .download_context(source, &command, Context::None)?;
        }

        Ok(())
    }
}
