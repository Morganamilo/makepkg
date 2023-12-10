mod curl;
mod file;
mod git;
mod vcs;

pub use vcs::*;

use std::{
    collections::BTreeMap, fs::remove_file, os::unix::fs::symlink, path::PathBuf, process::Command,
};

use crate::{
    callback::Event,
    config::{DownloadAgent, PkgbuildDirs, VCSClient},
    error::{CommandErrorExt, Context, DownloadError, IOContext, IOErrorExt, Result},
    fs::{mkdir, set_time},
    options::Options,
    pkgbuild::{Function, Pkgbuild, Source},
    Makepkg,
};

impl Makepkg {
    pub fn download_sources(
        &self,
        options: &Options,
        pkgbuild: &Pkgbuild,
        all: bool,
    ) -> Result<()> {
        self.event(Event::RetrievingSources);
        let dirs = self.pkgbuild_dirs(pkgbuild)?;

        mkdir(&dirs.srcdest, Context::RetrieveSources)?;

        let (mut downloads, vcs_downloads) = self.get_downloads(pkgbuild, &dirs, all)?;

        if let Some(curl) = downloads
            .keys()
            .copied()
            .find(|a| a.command.rsplit('/').next().unwrap() == "curl")
        {
            let curl = curl.clone();
            let sources = downloads.remove(&curl).unwrap();

            self.download_curl_sources(&dirs, sources)?;
        }

        self.download_file(&dirs, &downloads)?;
        self.download_vcs(&dirs, options, pkgbuild, &vcs_downloads)?;

        Ok(())
    }

    pub fn extract_sources(&self, options: &Options, pkgbuild: &Pkgbuild, all: bool) -> Result<()> {
        self.event(Event::ExtractingSources);

        let dirs = self.pkgbuild_dirs(pkgbuild)?;

        for source in &pkgbuild.source.values {
            if !all && !source.enabled(&self.config.arch) {
                continue;
            }

            for source in &source.values {
                match source.protocol() {
                    Some(proto) if is_vcs_proto(proto) => self.extract_vcs(&dirs, source)?,
                    _ => self.extract_file(&dirs, source, &pkgbuild.noextract)?,
                }
            }
        }

        if !options.no_prepare {
            self.run_function(options, pkgbuild, Function::Prepare)?
        }
        if options.reproducable {
            for file in walkdir::WalkDir::new(&dirs.srcdir) {
                let file = file.context(
                    Context::ExtractSources,
                    IOContext::ReadDir(dirs.srcdir.to_path_buf()),
                )?;
                set_time(file.path(), self.config.source_date_epoch)?;
            }
        }

        self.event(Event::SourcesAreReady);

        Ok(())
    }

    fn extract_file(
        &self,
        dirs: &PkgbuildDirs,
        source: &Source,
        noextract: &[String],
    ) -> Result<()> {
        let srcdestfile = self.download_path(dirs, source);
        let srcfile = dirs.srcdir.join(source.file_name());
        if srcfile.exists() {
            remove_file(&srcfile)
                .context(Context::ExtractSources, IOContext::Remove(srcfile.clone()))?;
        }
        symlink(&srcdestfile, &srcfile).context(
            Context::ExtractSources,
            IOContext::Link(srcdestfile.clone(), srcfile.clone()),
        )?;

        if noextract.iter().any(|s| s == source.file_name()) {
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

    fn get_downloads<'a>(
        &'a self,
        pkgbuild: &'a Pkgbuild,
        dirs: &PkgbuildDirs,
        all: bool,
    ) -> Result<(
        BTreeMap<&'a DownloadAgent, Vec<&'a Source>>,
        BTreeMap<&'a VCSClient, Vec<&'a Source>>,
    )> {
        let mut downloads: BTreeMap<&DownloadAgent, Vec<&Source>> = BTreeMap::new();
        let mut vcs_downloads: BTreeMap<&VCSClient, Vec<&Source>> = BTreeMap::new();

        let all_sources = if all {
            pkgbuild.source.all().collect::<Vec<_>>()
        } else {
            pkgbuild
                .source
                .enabled(&self.config.arch)
                .collect::<Vec<_>>()
        };

        if all_sources.is_empty() {
            return Ok(Default::default());
        }

        for source in all_sources {
            let path = self.download_path(dirs, source);

            if let Some(tool) = self.get_vcs_tool(&source) {
                vcs_downloads.entry(tool).or_default().push(source);
            } else if path.exists() {
                self.event(Event::FoundSource(source.file_name().to_string()));
                continue;
            } else if !source.is_download() {
                return Err(DownloadError::SourceMissing(source.clone()).into());
            } else if let Some(tool) = self.get_download_tool(&source) {
                downloads.entry(tool).or_default().push(source);
            } else {
                return Err(DownloadError::UnknownProtocol(source.clone()).into());
            }
        }

        Ok((downloads, vcs_downloads))
    }

    fn get_download_tool(&self, source: &Source) -> Option<&DownloadAgent> {
        let download_proto = source.protocol()?;
        self.config
            .dl_agents
            .iter()
            .find(|a| a.protocol == download_proto)
    }

    pub fn download_path(&self, dirs: &PkgbuildDirs, source: &Source) -> PathBuf {
        if source.is_download() {
            dirs.srcdest.join(source.file_name())
        } else {
            dirs.startdir.join(source.file_name())
        }
    }
}
