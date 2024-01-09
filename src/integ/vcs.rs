use digest::Digest;

use crate::{
    config::PkgbuildDirs,
    error::{IntegError, Result},
    pkgbuild::Source,
    sources::VCSKind,
    Makepkg,
};

impl Makepkg {
    pub(crate) fn checksum_vcs<D: Digest>(
        &self,
        dirs: &PkgbuildDirs,
        vcs: VCSKind,
        source: &Source,
    ) -> Result<String> {
        match vcs {
            //Some("git") => self.checksum_git::<D>(source),
            VCSKind::Mercurial => self.checksum_hg::<D>(dirs, source),
            //Some("bzr") => self.checksum_bzr::<D>(source),
            _ => Err(IntegError::DoesNotSupportChecksums(source.clone()).into()),
        }
    }
}
