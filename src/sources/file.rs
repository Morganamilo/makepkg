use std::{collections::BTreeMap, process::Command};

use crate::{
    callback::Event,
    config::{DownloadAgent, PkgbuildDirs},
    error::{CommandErrorExt, Context, Result},
    fs::{make_link, rename, rm_file},
    pkgbuild::Source,
    Makepkg,
};

impl Makepkg {
    pub(crate) fn download_file(
        &self,
        dirs: &PkgbuildDirs,
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

                self.event(Event::Downloading(source.file_name().to_string()));
                let mut command = Command::new(&agent.command);
                command
                    .args(&args)
                    .current_dir(&dirs.srcdest)
                    .status()
                    .download_context(source, &command, Context::None)?;

                rename(&part, &final_path, Context::RetrieveSources)?;
            }
        }
        Ok(())
    }

    pub(crate) fn extract_file(
        &self,
        dirs: &PkgbuildDirs,
        source: &Source,
        no_extract: &[String],
    ) -> Result<()> {
        let srcdestfile = dirs.download_path(source);
        let srcfile = dirs.srcdir.join(source.file_name());
        if srcfile.exists() {
            rm_file(&srcfile, Context::ExtractSources)?;
        }

        make_link(&srcdestfile, &srcfile, Context::ExtractSources)?;

        if no_extract.iter().any(|s| s == source.file_name()) {
            self.event(Event::NoExtact(source.file_name().to_string()));
            return Ok(());
        }

        // TODO more tarball kinds
        let supported = Command::new("bsdtar")
            .arg("-tf")
            .arg(&srcfile)
            .output()
            .ok()
            .map(|o| o.status.success())
            .unwrap_or(false);

        if supported {
            self.event(Event::Extacting(source.file_name().to_string()));
            let mut command = Command::new("bsdtar");
            command
                .arg("-xf")
                .arg(&srcfile)
                .current_dir(&dirs.srcdir)
                .status()
                .cmd_context(&command, Context::ExtractSources)?;
        }

        Ok(())
    }
}
