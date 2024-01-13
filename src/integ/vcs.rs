use std::io::Write;

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

    pub(crate) fn checksum_vcs<D: Digest + Write>(
        &self,
        dirs: &PkgbuildDirs,
        pkgbuild: &Pkgbuild,
        vcs: VCSKind,
        source: &Source,
    ) -> Result<String> {
        match vcs {
            VCSKind::Git => self.checksum_git::<D>(dirs, pkgbuild, source),
            VCSKind::Mercurial => self.checksum_hg::<D>(dirs, pkgbuild, source),
            VCSKind::Bzr => self.checksum_bzr::<D>(dirs, pkgbuild, source),
            _ => Err(IntegError::DoesNotSupportChecksums(source.clone()).into()),
        }
    }
}
