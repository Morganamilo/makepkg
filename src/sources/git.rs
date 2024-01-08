use std::process::Command;

use crate::{
    callback::Event,
    config::PkgbuildDirs,
    error::{CommandErrorExt, Context, DownloadError, Result},
    pkgbuild::{Fragment, Source},
    sources::VCSKind,
    Makepkg, Options, TOOL_NAME,
};

impl Makepkg {
    pub(crate) fn download_git(
        &self,
        dirs: &PkgbuildDirs,
        options: &Options,
        source: &Source,
    ) -> Result<()> {
        let path = dirs.download_path(source);

        if !path.exists() || !path.join("objects").exists() {
            println!("    cloning {} git repo...", source.file_name());

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
                .env("GIT_TERMINAL_PROMPT", "0");
            let status = command.status();
            status.download_context(&source, &command, Context::None)?;
        } else if !options.holdver {
            let mut command = Command::new("git");
            command
                .arg("config")
                .arg("--get")
                .arg("remote.origin.url")
                .current_dir(dirs.download_path(source));
            let remote_url = command.output();
            let remote_url = remote_url.download_context(&source, &command, Context::None)?;
            let remote_url = String::from_utf8(remote_url.stdout)
                .download_context(&source, &command, Context::None)?
                .trim()
                .to_string();

            if remote_url.trim_end_matches(".git") != source.url.trim_end_matches(".git") {
                return Err(DownloadError::RemotesDiffer(source.clone()).into());
            }

            self.event(Event::UpdatingVCS(VCSKind::Git, source.clone()));

            let mut command = Command::new("git");
            command
                .arg("fetch")
                .arg("--all")
                .arg("-p")
                .env("GIT_TERMINAL_PROMPT", "0")
                .current_dir(dirs.download_path(source));
            command
                .status()
                .download_context(&source, &command, Context::None)?;
        }

        Ok(())
    }

    pub(crate) fn extract_git(&self, dirs: &PkgbuildDirs, source: &Source) -> Result<()> {
        let mut gitref = "origin/HEAD".to_string();
        let mut updating = false;
        let srcpath = dirs.srcdir.join(source.file_name());
        //println!("    creating working copy of {} git repo...", source.path());

        if srcpath.exists() {
            updating = true;
            let mut command = Command::new("git");
            command
                .arg("fetch")
                .current_dir(&srcpath)
                .status()
                .download_context(&source, &command, Context::None)?;
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
                .status()
                .download_context(&source, &command, Context::None)?;
        }

        match &source.fragment {
            Some(Fragment::Commit(r) | Fragment::Tag(r)) => gitref = r.to_string(),
            Some(Fragment::Branch(r)) => gitref = format!("origin/{}", r),
            Some(f) => panic!("git does not support fragment {}", f),
            _ => (),
        }

        if matches!(source.fragment, Some(Fragment::Tag(_))) {
            let mut command = Command::new("git");
            command
                .arg("tag")
                .arg("-l")
                .arg("--format=%(tag)")
                .arg(&gitref)
                .arg(&srcpath);
            let tagname = command
                .output()
                .download_context(&source, &command, Context::None)?;
            let tagname = String::from_utf8(tagname.stdout)
                .download_context(&source, &command, Context::None)?
                .trim()
                .to_string();

            if !tagname.is_empty() && tagname != gitref {
                panic!(
                    "failed to checkout version {}, the git tag has been forged",
                    gitref
                );
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
                .status()
                .download_context(&source, &command, Context::None)?;
        }

        Ok(())
    }
}
