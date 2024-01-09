use std::{collections::BTreeMap, fmt::Display, str::FromStr};

use crate::{
    config::PkgbuildDirs,
    error::{Result, VCSClientError},
    pkgbuild::{Pkgbuild, Source},
    Makepkg, Options,
};

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum VCSKind {
    Git,
    Svn,
    Mercurial,
    Fossil,
    Bzr,
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
            VCSKind::Svn => "svn",
            VCSKind::Mercurial => "hg",
            VCSKind::Fossil => "fossil",
            VCSKind::Bzr => "bzr",
        }
    }
}

impl FromStr for VCSKind {
    type Err = VCSClientError;

    fn from_str(s: &str) -> std::prelude::v1::Result<Self, Self::Err> {
        match s {
            "git" => Ok(VCSKind::Git),
            "svn" => Ok(VCSKind::Svn),
            "hg" => Ok(VCSKind::Mercurial),
            "fossil" => Ok(VCSKind::Fossil),
            "bzr" => Ok(VCSKind::Bzr),
            _ => Err(VCSClientError { input: s.into() }),
        }
    }
}

impl Source {
    pub fn vcs_kind(&self) -> Option<VCSKind> {
        self.protocol().and_then(|p| p.parse().ok())
    }
}

impl Makepkg {
    pub(crate) fn extract_vcs(
        &self,
        dirs: &PkgbuildDirs,
        vcs: VCSKind,
        source: &Source,
    ) -> Result<()> {
        match vcs {
            VCSKind::Git => self.extract_git(dirs, source),
            VCSKind::Svn => self.extract_svn(dirs, source),
            VCSKind::Mercurial => self.extract_hg(dirs, source),
            VCSKind::Fossil => self.extract_fossil(dirs, source),
            VCSKind::Bzr => self.extract_bzr(dirs, source),
        }
    }

    pub(crate) fn download_vcs(
        &self,
        dirs: &PkgbuildDirs,
        options: &Options,
        _pkgbuild: &Pkgbuild,
        sources: &BTreeMap<VCSKind, Vec<&Source>>,
    ) -> Result<()> {
        for (vcs, sources) in sources {
            for &source in sources {
                match vcs {
                    VCSKind::Git => self.download_git(dirs, options, source)?,
                    VCSKind::Svn => self.download_svn(dirs, options, source)?,
                    VCSKind::Mercurial => self.download_hg(dirs, options, source)?,
                    VCSKind::Fossil => self.download_fossil(dirs, options, source)?,
                    VCSKind::Bzr => self.download_bzr(dirs, options, source)?,
                }
            }
        }
        Ok(())
    }
}
