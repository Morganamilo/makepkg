use std::{collections::BTreeMap, process::Command};

use crate::{
    callback::Event,
    config::{DownloadAgent, PkgbuildDirs},
    error::{CommandErrorExt, Context, Result},
    fs::{make_link, rename, rm_file},
    pkgbuild::{Pkgbuild, Source},
    run::CommandOutput,
    CommandKind, Makepkg,
};

impl Makepkg {
    pub(crate) fn download_file(
        &self,
        dirs: &PkgbuildDirs,
        pkgbuild: &Pkgbuild,
        downloads: &BTreeMap<&DownloadAgent, Vec<&Source>>,
    ) -> Result<()> {
        for (agent, sources) in downloads {
            for source in sources {
                let final_path = dirs.download_path(source).display().to_string();
                let part = format!("{}.part", final_path);
                let url = source.url.as_str();
                let url = url.trim_start_matches("scp://");

                let mut args = agent.args.clone();
                if !args.iter_mut().any(|s| s.contains("%u")) {
                    args.push(url.to_string());
                }

                for arg in &mut args {
                    *arg = arg.replace("%u", url);
                    *arg = arg.replace("%o", &part);
                }

                self.event(Event::Downloading(source.file_name()))?;
                let mut command = Command::new(&agent.command);
                command
                    .args(&args)
                    .current_dir(&dirs.srcdest)
                    .process_spawn(self, CommandKind::DownloadSources(pkgbuild, source))
                    .download_context(source, &command, Context::None)?;

                rename(&part, &final_path, Context::RetrieveSources)?;
            }
        }
        Ok(())
    }

    pub(crate) fn extract_file(
        &self,
        dirs: &PkgbuildDirs,
        pkgbuild: &Pkgbuild,
        source: &Source,
    ) -> Result<()> {
        let srcdestfile = dirs.download_path(source);
        let srcfile = dirs.srcdir.join(source.file_name());
        if srcfile.exists() {
            rm_file(&srcfile, Context::ExtractSources)?;
        }

        make_link(srcdestfile, &srcfile, Context::ExtractSources)?;

        if pkgbuild.noextract.iter().any(|s| s == source.file_name()) {
            self.event(Event::NoExtact(source.file_name()))?;
            return Ok(());
        }

        // TODO more tarball kinds
        let supported = Command::new("bsdtar")
            .arg("-tf")
            .arg(&srcfile)
            .process_output()
            .ok()
            .map(|s| s.status.success())
            .unwrap_or(false);

        if supported {
            self.event(Event::Extacting(source.file_name()))?;
            let mut command = Command::new("bsdtar");
            command
                .arg("-xf")
                .arg(&srcfile)
                .current_dir(&dirs.srcdir)
                .process_spawn(self, CommandKind::ExtractSources(pkgbuild, source))
                .cmd_context(&command, Context::ExtractSources)?;
        }

        Ok(())
    }
}
