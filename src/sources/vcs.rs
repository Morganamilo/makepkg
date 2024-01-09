use std::{collections::BTreeMap, fmt::Display};

use crate::{
    config::{PkgbuildDirs, VCSClient},
    error::{DownloadError, Result},
    pkgbuild::{Pkgbuild, Source},
    Makepkg, Options,
};

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum VCSKind {
    Git,
    SVN,
    Mercurial,
    Fossil,
    BZR,
}

impl Display for VCSKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

impl VCSKind {
    pub fn name(&self) -> &'static str {
        match self {
            VCSKind::Git => "git",
            VCSKind::SVN => "svn",
            VCSKind::Mercurial => "hg",
            VCSKind::Fossil => "fossil",
            VCSKind::BZR => todo!("bzr"),
        }
    }
}

impl Source {
    pub fn vcs_proto(&self) -> Option<VCSKind> {
        match self.protocol() {
            Some("git") => Some(VCSKind::Git),
            Some("svn") => Some(VCSKind::SVN),
            Some("hg") => Some(VCSKind::Mercurial),
            Some("fossil") => Some(VCSKind::Fossil),
            Some("bzr") => Some(VCSKind::BZR),
            _ => None,
        }
    }
}

impl Makepkg {
    pub(crate) fn extract_vcs(&self, dirs: &PkgbuildDirs, source: &Source) -> Result<()> {
        match source.protocol() {
            Some("git") => self.extract_git(dirs, source),
            Some("svn") => self.extract_svn(dirs, source),
            Some("hg") => self.extract_hg(dirs, source),
            Some("fossil") => self.extract_fossil(dirs, source),
            Some("bzr") => self.extract_bzr(dirs, source),
            _ => return Err(DownloadError::UnknownVCSClient(source.clone()).into()),
        }
    }

    pub(crate) fn download_vcs(
        &self,
        dirs: &PkgbuildDirs,
        options: &Options,
        _pkgbuild: &Pkgbuild,
        sources: &BTreeMap<&VCSClient, Vec<&Source>>,
    ) -> Result<()> {
        for (client, sources) in sources {
            for &source in sources {
                match client.protocol.as_str() {
                    "git" => self.download_git(dirs, options, source)?,
                    "svn" => self.download_svn(dirs, options, source)?,
                    "hg" => self.download_hg(dirs, options, source)?,
                    "fossil" => self.download_fossil(dirs, options, source)?,
                    "bzr" => self.download_bzr(dirs, options, source)?,
                    _ => return Err(DownloadError::UnknownVCSClient(source.clone()).into()),
                }
            }
        }
        Ok(())
    }

    pub(crate) fn get_vcs_tool(&self, source: &Source) -> Option<&VCSClient> {
        let download_proto = source.protocol()?;

        self.config
            .vcs_agents
            .iter()
            .find(|a| a.protocol == download_proto)
    }
}
