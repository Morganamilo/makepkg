use std::{
    fs::File,
    io::{self, stdout, ErrorKind, Read, Write},
    ops::Deref,
    os::{
        fd::{AsRawFd, OwnedFd},
        unix::net::UnixStream,
    },
    path::Path,
    process::{Command, Stdio},
};

use nix::{
    errno::Errno,
    poll::{poll, PollFd, PollFlags},
};

use crate::{
    callback::Event,
    config::PkgbuildDirs,
    error::{CommandErrorExt, CommandOutputExt, Context, IOContext, IOError, IOErrorExt, Result},
    fs::open,
    installation_variables::FAKEROOT_LIBDIRS,
    makepkg::FakeRoot,
    options::Options,
    pkgbuild::{Function, Pkgbuild},
    raw::PKGBUILD_SCRIPT,
    Makepkg,
};

fn pipe(function: &str) -> Result<(UnixStream, UnixStream)> {
    let (r, w) =
        UnixStream::pair().context(Context::RunFunction(function.to_string()), IOContext::Pipe)?;
    r.set_nonblocking(true)
        .context(Context::RunFunction(function.to_string()), IOContext::Pipe)?;

    Ok((r, w))
}

impl Makepkg {
    pub fn update_pkgver(&self, options: &Options, pkgbuild: &mut Pkgbuild) -> Result<()> {
        if !pkgbuild.has_function(Function::Pkgver) {
            return Ok(());
        }

        let dirs = self.pkgbuild_dirs(pkgbuild)?;
        let pkgver = self.run_function_internal(
            options,
            &dirs,
            pkgbuild,
            None,
            Function::Pkgver.name(),
            true,
        )?;
        pkgbuild.set_pkgver(&dirs.pkgbuild, pkgver)
    }

    pub fn run_function(
        &self,
        options: &Options,
        pkgbuild: &Pkgbuild,
        function: Function,
    ) -> Result<()> {
        let dirs = self.pkgbuild_dirs(pkgbuild)?;

        if !pkgbuild.has_function(function) {
            return Ok(());
        }

        if function == Function::Package {
            for function in &pkgbuild.package_functions {
                if function == "package" {
                    self.run_function_internal(
                        options,
                        &dirs,
                        pkgbuild,
                        Some(pkgbuild.packages[0].pkgname.as_str()),
                        function,
                        false,
                    )?;
                } else {
                    let pkgname = Some(function.trim_start_matches("package_"));
                    self.run_function_internal(options, &dirs, pkgbuild, pkgname, function, false)?;
                }
            }
        } else if function == Function::Pkgver {
            self.run_function_internal(options, &dirs, pkgbuild, None, function.name(), true)?;
        } else {
            self.run_function_internal(options, &dirs, pkgbuild, None, function.name(), false)?;
        }
        Ok(())
    }

    fn run_function_internal(
        &self,
        options: &Options,
        dirs: &PkgbuildDirs,
        pkgbuild: &Pkgbuild,
        pkgname: Option<&str>,
        function: &str,
        capture_output: bool,
    ) -> Result<String> {
        self.event(Event::RunningFunction(function.to_string()));
        //let capture_output = true;
        let mut options = options.clone();
        //options.log = true;

        let workingdir = match function {
            "verify" => dirs.startdir.as_path(),
            _ => dirs.srcdir.as_path(),
        };

        let pkgbase = pkgbuild.pkgbase.as_str();
        let version = pkgbuild.version();
        let pkgdir = &dirs.pkgdir.join(pkgname.unwrap_or(pkgbase));

        let mut outputfd = 0;
        let mut readers = Vec::new();
        let mut output = Vec::new();
        let mut buffer = vec![0; 512];

        let mut command = Command::new("bash");
        command
            .arg("--noprofile")
            .arg("--norc")
            .arg("-s")
            .arg("-")
            .arg("run")
            .arg(&dirs.pkgbuild)
            .arg(workingdir)
            .arg(function)
            .env("CARCH", &self.config.arch)
            .env("startdir", &dirs.startdir)
            .env("srcdir", &dirs.srcdir)
            .env("pkgdir", pkgdir)
            .current_dir(&dirs.startdir)
            .stdin(Stdio::piped());

        if matches!(function, "build" | "check") || function.starts_with("package") {
            self.build_env(dirs, pkgbuild, &mut command);
        }
        if function.starts_with("package") {
            self.fakeroot_env(&mut command)?;
        }
        if let Some(pkgname) = pkgname {
            command.arg(pkgname);
        }

        let mut logfile = if options.log {
            let logfile = dirs.logdest.join(format!(
                "{}-{}-{}-{}.log",
                pkgbase, version, self.config.arch, function,
            ));

            let mut file = File::options();
            let file = file.create(true).truncate(true).write(true);
            let file = open(file, logfile, Context::RunFunction(function.to_string()))?;
            Some(file)
        } else {
            None
        };

        if capture_output || options.log {
            let (read1, write1) = pipe(function)?;

            let write2 = if capture_output {
                let (read2, write2) = pipe(function)?;
                readers.push(read2);
                outputfd = read1.as_raw_fd();
                write2
            } else {
                write1
                    .try_clone()
                    .context(Context::RunFunction(function.to_string()), IOContext::Dup)?
            };

            command.stdout(OwnedFd::from(write1));
            command.stderr(OwnedFd::from(write2));
            readers.push(read1);
        }

        let mut child = command
            .spawn()
            .cmd_context(&command, Context::RunFunction(function.to_string()))?;

        let mut stdin = child.stdin.take().unwrap();
        stdin
            .write_all(PKGBUILD_SCRIPT.as_bytes())
            .cmd_context(&command, Context::RunFunction(function.to_string()))?;

        drop(stdin);
        let mut stdout = stdout().lock();
        command.stdout(Stdio::null());
        command.stderr(Stdio::null());

        while !readers.is_empty() {
            let mut res;
            let mut pollfds = readers
                .iter()
                .map(|fd| PollFd::new(fd, PollFlags::POLLIN))
                .collect::<Vec<_>>();

            loop {
                res = poll(&mut pollfds, -1);
                if !matches!(res, Err(Errno::EAGAIN | Errno::EINTR)) {
                    break;
                }
            }

            res.context(Context::RunFunction(function.to_string()), IOContext::Pipe)?;

            let mut events = pollfds
                .into_iter()
                .map(|e| e.revents().unwrap())
                .collect::<Vec<_>>();

            for (i, event) in events.iter_mut().enumerate().rev() {
                if event.intersects(PollFlags::POLLIN) {
                    write_output(
                        &mut readers[i],
                        &mut buffer,
                        outputfd,
                        &mut output,
                        &mut logfile,
                        &mut stdout,
                        event,
                    )
                    .context(
                        Context::RunFunction(function.to_string()),
                        IOContext::WriteBuffer,
                    )?;
                }

                if event.intersects(PollFlags::POLLERR | PollFlags::POLLNVAL | PollFlags::POLLHUP) {
                    readers.remove(i);
                    continue;
                }
            }
        }

        child
            .wait()
            .cmd_context(&command, Context::RunFunction(function.to_string()))?;

        let output = output.read(&command, Context::RunFunction(function.to_string()))?;
        Ok(output)
    }

    pub(crate) fn fakeroot(&self) -> Result<String> {
        let mut fakeroot = self.fakeroot.borrow_mut();

        if let Some(fakeroot) = fakeroot.deref() {
            return Ok(fakeroot.key.clone());
        }

        self.event(Event::StartingFakeroot);

        if !FAKEROOT_LIBDIRS
            .split(':')
            .any(|dir| Path::new(dir).join(FakeRoot::library_name()).exists())
        {
            return Err(IOError::new(
                Context::StartFakeroot,
                IOContext::FindLibfakeroot(FAKEROOT_LIBDIRS.split(':').map(Into::into).collect()),
                io::ErrorKind::Other,
            )
            .into());
        }

        let mut key = [0; 50];
        let mut command = Command::new("faked");
        let mut child = command
            .arg("--foreground")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .cmd_context(&command, Context::StartFakeroot)?;

        let mut stdout = child.stdout.take().unwrap();
        let n = stdout.read(&mut key).unwrap();
        let key = std::str::from_utf8(&key[0..n]).unwrap();
        let key = key.split_once(':').unwrap().0.to_string();
        let ret = key.clone();

        let newfakeroot = FakeRoot { key, child };
        *fakeroot = Some(newfakeroot);
        Ok(ret)
    }
}

fn write_output(
    sock: &mut UnixStream,
    buffer: &mut [u8],
    outputfd: i32,
    output: &mut Vec<u8>,
    logfile: &mut Option<File>,
    stdout: &mut io::StdoutLock<'_>,
    event: &mut PollFlags,
) -> io::Result<()> {
    loop {
        match sock.read(buffer) {
            Ok(0) => {
                *event = PollFlags::POLLERR;
                return Ok(());
            }
            Ok(n) => {
                stdout.write_all(&buffer[..n])?;
                if outputfd == sock.as_raw_fd() {
                    output.extend(&buffer[..n])
                }
                if let Some(log) = logfile {
                    log.write_all(&buffer[..n])?;
                }
            }
            Err(e) if e.kind() == ErrorKind::WouldBlock => {
                panic!("we should never get this");
            }
            Err(e) => return Err(e),
        }
        if !event.contains(PollFlags::POLLHUP) {
            return Ok(());
        }
    }
}
