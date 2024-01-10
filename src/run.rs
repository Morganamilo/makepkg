use std::{
    fs::File,
    io::{self, stdout, ErrorKind, Read, Write},
    ops::Deref,
    os::{fd::OwnedFd, unix::net::UnixStream},
    path::Path,
    process::{Command, Stdio},
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

        let pkgbase = pkgbuild.pkgbase.as_str();
        let pkgdir = &dirs.pkgdir.join(pkgname.unwrap_or(pkgbase));

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
                pkgbuild.pkgbase,
                pkgbuild.version(),
                self.config.arch,
                function,
            ));

            let mut file = File::options();
            let file = file.create(true).truncate(true).write(true);
            let file = open(file, logfile, Context::RunFunction(function.to_string()))?;
            Some(file)
        } else {
            None
        };

        let mut readers = Vec::new();
        let mut output = Vec::new();
        let mut buffer = vec![0; 512];
        let mut buffer = buffer.as_mut_slice();

        if capture_output {
            let (read1, write1) = pipe(function)?;
            let (read2, write2) = pipe(function)?;
            command.stdout(OwnedFd::from(write1));
            command.stderr(OwnedFd::from(write2));
            readers.push((read1, true));
            readers.push((read2, false));
        } else if options.log {
            let (read1, write1) = pipe(function)?;
            let write2 = write1
                .try_clone()
                .context(Context::RunFunction(function.to_string()), IOContext::Dup)?;
            command.stdout(OwnedFd::from(write1));
            command.stderr(OwnedFd::from(write2));
            readers.push((read1, false));
        }

        let mut child = command
            .spawn()
            .cmd_context(&command, Context::RunFunction(function.to_string()))?;

        let mut stdin = child.stdin.take().unwrap();

        stdin
            .write_all(PKGBUILD_SCRIPT.as_bytes())
            .cmd_context(&command, Context::RunFunction(function.to_string()))?;

        let mut stdout = stdout().lock();
        drop(stdin);
        command.stdout(Stdio::null());
        command.stderr(Stdio::null());

        while !readers.is_empty() {
            for (i, (reader, is_stdout)) in readers.iter_mut().enumerate() {
                match reader.read(&mut buffer) {
                    Ok(0) => {
                        readers.remove(i);
                        break;
                    }
                    Ok(n) => {
                        if *is_stdout && capture_output {
                            output.extend(&buffer[..n]);
                        }
                        if let Some(log) = &mut logfile {
                            log.write_all(&buffer[..n]).cmd_context(
                                &command,
                                Context::RunFunction(function.to_string()),
                            )?;
                        }

                        stdout
                            .write_all(&buffer[..n])
                            .cmd_context(&command, Context::RunFunction(function.to_string()))?;
                    }
                    Err(e) if e.kind() == ErrorKind::WouldBlock => continue,
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
