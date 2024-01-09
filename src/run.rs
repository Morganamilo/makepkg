use std::{
    fs::File,
    io::{self, stdout, ErrorKind, Read, Write},
    ops::Deref,
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
    path::Path,
    process::{Command, Stdio},
};

use nix::{
    fcntl::{fcntl, FcntlArg, OFlag},
    unistd::{self, close},
};

use crate::{
    callback::Event,
    config::PkgbuildDirs,
    error::{
        CommandError, CommandErrorExt, CommandOutputExt, Context, IOContext, IOError, IOErrorExt,
        Result,
    },
    fs::open,
    installation_variables::FAKEROOT_LIBDIRS,
    makepkg::FakeRoot,
    options::Options,
    pkgbuild::{Function, Pkgbuild},
    raw::PKGBUILD_SCRIPT,
    Makepkg,
};

fn pipe(function: &str) -> Result<(OwnedFd, OwnedFd)> {
    let (r, w) =
        unistd::pipe().context(Context::RunFunction(function.to_string()), IOContext::Pipe)?;
    fcntl(r, FcntlArg::F_SETFL(OFlag::O_NONBLOCK))
        .context(Context::RunFunction(function.to_string()), IOContext::Pipe)?;
    unsafe { Ok((OwnedFd::from_raw_fd(r), OwnedFd::from_raw_fd(w))) }
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
            &dirs.srcdir,
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

        let workingdir = match function {
            Function::Verify => dirs.startdir.as_path(),
            _ => dirs.srcdir.as_path(),
        };

        if function == Function::Package {
            for function in &pkgbuild.package_functions {
                if function == "package" {
                    self.run_function_internal(
                        options,
                        &dirs,
                        pkgbuild,
                        workingdir,
                        Some(pkgbuild.packages[0].pkgname.as_str()),
                        function,
                        false,
                    )?;
                } else {
                    let pkgname = Some(function.trim_start_matches("package_"));
                    self.run_function_internal(
                        options, &dirs, pkgbuild, workingdir, pkgname, function, false,
                    )?;
                }
            }
        } else if function == Function::Pkgver {
            self.run_function_internal(
                options,
                &dirs,
                pkgbuild,
                workingdir,
                None,
                function.name(),
                true,
            )?;
        } else {
            self.run_function_internal(
                options,
                &dirs,
                pkgbuild,
                workingdir,
                None,
                function.name(),
                false,
            )?;
        }
        Ok(())
    }

    fn run_function_internal(
        &self,
        options: &Options,
        dirs: &PkgbuildDirs,
        pkgbuild: &Pkgbuild,
        workingdir: &Path,
        pkgname: Option<&str>,
        function: &str,
        capture_output: bool,
    ) -> Result<String> {
        self.event(Event::RunningFunction(function.to_string()));

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
            .env(
                "pkgdir",
                &dirs
                    .pkgdir
                    .join(pkgname.unwrap_or(pkgbuild.pkgbase.as_str())),
            )
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

        let logfile = dirs.logdest.join(format!(
            "{}-{}-{}-{}.log",
            pkgbuild.pkgbase,
            pkgbuild.version(),
            self.config.arch,
            function,
        ));

        let mut logfile = if options.log {
            let mut file = File::options();
            let file = file.create(true).truncate(true).write(true);
            let file = open(file, logfile, Context::RunFunction(function.to_string()))?;
            Some(file)
        } else {
            None
        };

        let mut reader_count = 0;
        let mut reader1 = None;
        let mut reader2 = None;
        let mut output = Vec::new();
        let mut buffer = vec![0; 512];
        let mut fds = None;

        if options.log || capture_output {
            let (read1, write1) = pipe(function)?;
            let read1 = File::from(read1);

            if !capture_output {
                let write2 = write1
                    .try_clone()
                    .context(Context::RunFunction(function.to_string()), IOContext::Dup)?;
                fds = Some((write1.as_raw_fd(), write2.as_raw_fd()));
                command.stderr(write2);
            } else {
                let (read2, write2) = pipe(function)?;
                let read2 = File::from(read2);
                fds = Some((write1.as_raw_fd(), write2.as_raw_fd()));
                command.stderr(write2);
                reader2 = Some(read2);
                reader_count += 1;
            }

            command.stdout(write1);
            reader1 = Some(read1);
            reader_count += 1;
        }

        let mut child = command
            .spawn()
            .cmd_context(&command, Context::RunFunction(function.to_string()))?;

        if let Some((fd1, fd2)) = fds {
            let _ = close(fd1);
            let _ = close(fd2);
        }

        let mut stdin = child.stdin.take().unwrap();

        stdin
            .write_all(PKGBUILD_SCRIPT.as_bytes())
            .cmd_context(&command, Context::RunFunction(function.to_string()))?;

        drop(stdin);

        let mut stdout = stdout().lock();

        if reader1.is_some() {
            loop {
                let mut done = 0;
                let readers = [&mut reader1, &mut reader2];
                for (i, reader) in readers.into_iter().flatten().enumerate() {
                    loop {
                        match reader.read(&mut buffer) {
                            Ok(0) => {
                                done += 1;
                                break;
                            }
                            Ok(n) => {
                                if i == 0 && capture_output {
                                    output.extend(&buffer[..n]);
                                }
                                if let Some(log) = &mut logfile {
                                    log.write_all(&buffer[..n]).cmd_context(
                                        &command,
                                        Context::RunFunction(function.to_string()),
                                    )?;
                                }
                                stdout.write_all(&buffer[..n]).cmd_context(
                                    &command,
                                    Context::RunFunction(function.to_string()),
                                )?;
                            }
                            Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                            Err(e) => {
                                return Err(CommandError::exec(
                                    e,
                                    &command,
                                    Context::RunFunction(function.to_string()),
                                )
                                .into());
                            }
                        }
                    }
                }

                if done == reader_count {
                    break;
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
