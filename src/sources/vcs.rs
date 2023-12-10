use std::{collections::BTreeMap, fmt::Display};

use crate::{
    config::{PkgbuildDirs, VCSClient},
    error::Result,
    pkgbuild::{Pkgbuild, Source},
    Makepkg, Options,
};

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum VCSKind {
    Git,
}

impl Display for VCSKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VCSKind::Git => f.write_str("git"),
        }
    }
}

pub fn is_vcs_proto(proto: &str) -> bool {
    ["bzr", "fossil", "git", "hg", "svn"].contains(&proto)
}

impl Makepkg {
    pub(crate) fn extract_vcs(&self, dirs: &PkgbuildDirs, source: &Source) -> Result<()> {
        match source.protocol() {
            Some("git") => self.extract_git(dirs, source),
            proto => panic!("unknown vcs protcol {}", proto.unwrap_or("none")),
        }
    }

    pub fn download_vcs(
        &self,
        dirs: &PkgbuildDirs,
        options: &Options,
        pkgbuild: &Pkgbuild,
        sources: &BTreeMap<&VCSClient, Vec<&Source>>,
    ) -> Result<()> {
        for (client, sources) in sources {
            //self.check_vcs_deps(pkgbuild, client)?;
            for source in sources {
                match client.protocol.as_str() {
                    "git" => self.download_git(dirs, options, source)?,
                    //"svn" => self.download_svn(source)?,
                    //"hg" => self.download_hg(source)?,
                    //"fossil" => self.download_fossil(source)?,
                    //"bzr" => self.download_bzr(source)?,
                    //_ => bail!("unknown vcs client {}", client.protocol),
                    _ => panic!("unknown vcs client {}", client.protocol),
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
