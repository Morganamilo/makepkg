use std::collections::BTreeMap;

pub use vcs::*;

type SourceMap<'a, T> = BTreeMap<T, Vec<&'a Source>>;

use crate::{
    callback::Event,
    config::{DownloadAgent, PkgbuildDirs},
    error::{Context, DownloadError, IOContext, IOErrorExt, Result},
    fs::{mkdir, set_time},
    options::Options,
    pkgbuild::{Function, Pkgbuild, Source},
    Makepkg,
};

mod bzr;
mod curl;
mod file;
mod fossil;
mod git;
mod mercurial;
mod svn;
mod vcs;

impl Makepkg {
    pub fn download_sources(
        &self,
        options: &Options,
        pkgbuild: &Pkgbuild,
        all: bool,
    ) -> Result<()> {
        self.event(Event::RetrievingSources)?;
        let dirs = self.pkgbuild_dirs(pkgbuild)?;

        mkdir(&dirs.srcdest, Context::RetrieveSources)?;

        let (downloads, vcs_downloads, curl_downloads) =
            self.get_downloads(pkgbuild, &dirs, all)?;

        self.download_curl_sources(&dirs, curl_downloads)?;
        self.download_file(&dirs, pkgbuild, &downloads)?;
        self.download_vcs(&dirs, options, pkgbuild, &vcs_downloads)?;

        Ok(())
    }

    pub fn extract_sources(&self, options: &Options, pkgbuild: &Pkgbuild, all: bool) -> Result<()> {
        self.event(Event::ExtractingSources)?;

        let dirs = self.pkgbuild_dirs(pkgbuild)?;

        for source in &pkgbuild.source.values {
            if !all && !source.enabled(&self.config.arch) {
                continue;
            }

            for source in &source.values {
                match source.vcs_kind() {
                    Some(vcs) => self.extract_vcs(&dirs, pkgbuild, vcs, source)?,
                    _ => self.extract_file(&dirs, pkgbuild, source)?,
                }
            }
        }

        if !options.no_prepare {
            self.run_function(options, pkgbuild, Function::Prepare)?
        }
        if options.reproducible {
            for file in walkdir::WalkDir::new(&dirs.srcdir) {
                let file = file.context(
                    Context::ExtractSources,
                    IOContext::ReadDir(dirs.srcdir.to_path_buf()),
                )?;
                set_time(file.path(), self.config.source_date_epoch, false)?;
            }
        }

        self.event(Event::SourcesAreReady)?;

        Ok(())
    }

    fn get_downloads<'a>(
        &'a self,
        pkgbuild: &'a Pkgbuild,
        dirs: &PkgbuildDirs,
        all: bool,
    ) -> Result<(
        SourceMap<&'a DownloadAgent>,
        SourceMap<VCSKind>,
        Vec<&'a Source>,
    )> {
        let mut downloads: SourceMap<&DownloadAgent> = BTreeMap::new();
        let mut vcs_downloads: SourceMap<VCSKind> = BTreeMap::new();
        let mut curl = Vec::new();

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
            let path = dirs.download_path(source);

            if let Some(tool) = source.vcs_kind() {
                vcs_downloads.entry(tool).or_default().push(source);
            } else if path.exists() {
                self.event(Event::FoundSource(source.file_name().to_string()))?;
                continue;
            } else if !source.is_remote() {
                return Err(DownloadError::SourceMissing(source.clone()).into());
            } else if let Some(tool) = self.get_download_tool(source) {
                if tool.command.rsplit('/').next().unwrap() == "curl" {
                    curl.push(source);
                } else {
                    downloads.entry(tool).or_default().push(source);
                }
            } else if self.curl_supports(source) {
                curl.push(source);
            } else {
                return Err(DownloadError::UnknownProtocol(source.clone()).into());
            }
        }

        Ok((downloads, vcs_downloads, curl))
    }

    fn curl_supports(&self, source: &Source) -> bool {
        let Some(protocol) = source.protocol() else {
            return false;
        };

        ::curl::Version::get().protocols().any(|p| p == protocol)
    }

    fn get_download_tool(&self, source: &Source) -> Option<&DownloadAgent> {
        let download_proto = source.protocol()?;
        self.config
            .dl_agents
            .iter()
            .find(|a| a.protocol == download_proto)
    }
}
