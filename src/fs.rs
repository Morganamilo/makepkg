use nix::sys::stat::{utimensat, UtimensatFlags};
use nix::sys::time::TimeSpec;

use crate::error::{Context, IOContext, IOError, IOErrorExt, Result};

use std::fs::{create_dir_all, remove_dir_all, File, OpenOptions};
use std::io::{self};
use std::os::unix::fs::PermissionsExt;
use std::path::{Component, PathBuf};

pub fn current_dir() -> Result<PathBuf> {
    let path = std::env::current_dir().context(Context::None, IOContext::CurrentDir)?;
    Ok(path)
}

use std::{fs::metadata, path::Path};

pub struct Check {
    context: Context,
    exists: bool,
    //file: bool,
}

impl Check {
    pub fn new(context: Context) -> Self {
        Check {
            context,
            exists: false,
            //file: false,
        }
    }

    pub fn read(mut self) -> Self {
        self.exists = true;
        self
    }

    pub fn check<P: AsRef<Path>>(self, path: P) -> Result<()> {
        let path = path.as_ref();

        match metadata(path) {
            //Ok(m) if self.file && !m.is_file() => {
            //    self.err(IOContext::NotAFile(path.into()), io::ErrorKind::Other)
            //}
            //Ok(m) if self.directory && !m.is_dir() => {
            //    self.err(path, IOAction::Directory, io::ErrorKind::Other)
            //}
            Err(e) if self.exists && e.kind() == io::ErrorKind::NotFound => {
                self.err(IOContext::NotFound(path.into()), io::ErrorKind::Other)
            }
            Err(e) => self.err(IOContext::Read(path.into()), e),
            Ok(_) => Ok(()),
        }
    }

    fn err<E: Into<io::Error>>(self, iocontext: IOContext, err: E) -> Result<()> {
        Err(IOError::new(self.context, iocontext, err.into()).into())
    }
}

pub fn resolve_path<P: AsRef<Path>>(path: P) -> Result<PathBuf> {
    let cwd = current_dir()?;
    Ok(resolve_path_relative(path, &cwd))
}

pub fn open<P: AsRef<Path>>(options: &OpenOptions, path: P, context: Context) -> Result<File> {
    let path = path.as_ref();
    let file = options
        .open(path)
        .context(context, IOContext::Open(path.into()))?;
    Ok(file)
}

pub fn mkdir<P: AsRef<Path>>(path: P, context: Context) -> Result<()> {
    let path = path.as_ref();
    create_dir_all(path).context(context, IOContext::Mkdir(path.into()))?;
    std::fs::set_permissions(path, PermissionsExt::from_mode(0o755))
        .context(Context::CreatePackage, IOContext::Chmod(path.into()))?;
    Ok(())
}

pub fn rm_all<P: AsRef<Path>>(path: P, context: Context) -> Result<()> {
    let path = path.as_ref();
    remove_dir_all(path).context(context, IOContext::Remove(path.into()))?;
    Ok(())
}

pub fn copy<P1: AsRef<Path>, P2: AsRef<Path>>(src: P1, dest: P2, context: Context) -> Result<()> {
    let (src, dest) = (src.as_ref(), dest.as_ref());
    std::fs::copy(src, dest).context(context, IOContext::Copy(src.into(), dest.into()))?;
    Ok(())
}

pub fn set_time<P: AsRef<Path>>(path: P, time: u64) -> Result<()> {
    let time = TimeSpec::new(time as _, 0);
    let path = path.as_ref();

    utimensat(None, path, &time, &time, UtimensatFlags::NoFollowSymlink)
        .context(Context::UnifySourceTime, IOContext::Utimensat(path.into()))?;
    Ok(())
}

pub fn resolve_path_relative<P1: AsRef<Path>, P2: AsRef<Path>>(path: P1, cwd: P2) -> PathBuf {
    let path = path.as_ref();
    let cwd = cwd.as_ref();
    let buf;

    let path = if path.is_absolute() {
        path
    } else {
        buf = cwd.join(path);
        buf.as_path()
    };

    let mut components = path.components().peekable();
    let mut ret = if let Some(c @ Component::Prefix(..)) = components.peek().cloned() {
        components.next();
        PathBuf::from(c.as_os_str())
    } else {
        PathBuf::new()
    };

    for component in components {
        match component {
            Component::Prefix(..) => unreachable!(),
            Component::RootDir => {
                ret.push(component.as_os_str());
            }
            Component::CurDir => {}
            Component::ParentDir => {
                ret.pop();
            }
            Component::Normal(c) => {
                ret.push(c);
            }
        }
    }

    ret
}
