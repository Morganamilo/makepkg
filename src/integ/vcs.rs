use digest::Digest;

use crate::{
    config::PkgbuildDirs,
    error::{IntegError, Result},
    pkgbuild::{Pkgbuild, Source},
    sources::VCSKind,
    Makepkg,
};

impl Makepkg {
    pub(crate) fn verify_vcs_sig(
        &self,
        dirs: &PkgbuildDirs,
        vcs: VCSKind,
        pkgbuild: &Pkgbuild,
        source: &Source,
        gpg: &mut gpgme::Context,
    ) -> Result<bool> {
        if source.query.as_deref() != Some("signed") {
            return Ok(true);
        }

        match vcs {
            VCSKind::Git => self.verify_git_sig(dirs, pkgbuild, source, gpg),
            _ => Err(IntegError::DoesNotSupportSignatures(source.clone()).into()),
        }
    }

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
