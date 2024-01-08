use std::{collections::BTreeMap, process::Command};

use crate::{
    callback::Event,
    config::{DownloadAgent, PkgbuildDirs},
    error::{CommandErrorExt, Context, Result},
    fs::rename,
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
}
