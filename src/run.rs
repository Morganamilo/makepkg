use std::{
    fs::File,
    io::{self, stdout, Empty, ErrorKind, Read, Write},
    net::Shutdown,
    ops::Deref,
    os::{
        fd::{AsFd, OwnedFd},
        unix::net::UnixStream,
    },
    path::Path,
    process::{Command, ExitStatus, Output, Stdio},
    result::Result as StdResult,
};

use mio::{Events, Interest, Poll, Token};

use crate::{
    callback::{self, CommandKind, Event},
    config::PkgbuildDirs,
    error::{CommandErrorExt, Context, IOContext, IOError, Result},
    fs::open,
    installation_variables::FAKEROOT_LIBDIRS,
    makepkg::FakeRoot,
    options::Options,
    pkgbuild::{Function, Pkgbuild},
    raw::PKGBUILD_SCRIPT,
    Makepkg,
};

pub(crate) trait CommandOutput {
    fn process_inner<W: Write>(
        &mut self,
        makepkg: &Makepkg,
        kind: CommandKind,
        input: &[u8],
        output: Option<&mut W>,
        ignore_stdout: bool,
        pipe_into: Option<&mut Command>,
        logfile: Option<&mut File>,
    ) -> StdResult<ExitStatus, io::Error>;
    fn process_pipe(
        &mut self,
        makepkg: &Makepkg,
        kind: CommandKind,
        input: &[u8],
        pipe_into: &mut Command,
    ) -> StdResult<ExitStatus, io::Error> {
        self.process_inner::<Empty>(makepkg, kind, input, None, true, Some(pipe_into), None)
    }
    fn process_function(
        &mut self,
        makepkg: &Makepkg,
        kind: CommandKind,
        input: &[u8],
        pkgver: Option<&mut Vec<u8>>,
        logfile: Option<&mut File>,
    ) -> StdResult<ExitStatus, io::Error> {
        self.process_inner(makepkg, kind, input, pkgver, false, None, logfile)
    }
    fn process_input_output<W: Write>(
        &mut self,
        makepkg: &Makepkg,
        kind: CommandKind,
        input: &[u8],
        output: Option<&mut W>,
    ) -> StdResult<ExitStatus, io::Error> {
        let ignore_stdout = output.is_some();
        self.process_inner(makepkg, kind, input, output, ignore_stdout, None, None)
    }
    fn process_write_output<W: Write>(
        &mut self,
        makepkg: &Makepkg,
        kind: CommandKind,
        output: &mut W,
    ) -> StdResult<ExitStatus, io::Error> {
        self.process_inner(makepkg, kind, &[], Some(output), true, None, None)
    }
    fn process_spawn(
        &mut self,
        makepkg: &Makepkg,
        kind: CommandKind,
    ) -> StdResult<ExitStatus, io::Error> {
        self.process_inner::<Empty>(makepkg, kind, &[], None, false, None, None)
    }
    fn process_read(
        &mut self,
        makepkg: &Makepkg,
        kind: CommandKind,
    ) -> StdResult<Output, io::Error> {
        let mut output = Vec::new();
        let output = Output {
            status: self.process_inner(makepkg, kind, &[], Some(&mut output), true, None, None)?,
            stdout: output,
            stderr: Vec::new(),
        };
        Ok(output)
    }
    fn process_output(&mut self) -> StdResult<Output, io::Error>;
}

impl CommandOutput for Command {
    fn process_output(&mut self) -> StdResult<Output, io::Error> {
        self.output()
    }

    fn process_inner<W: Write>(
        &mut self,
        makepkg: &Makepkg,
        kind: CommandKind,
        mut input: &[u8],
        mut output: Option<&mut W>,
        ignore_stdout: bool,
        pipe_into: Option<&mut Command>,
        mut logfile: Option<&mut File>,
    ) -> StdResult<ExitStatus, io::Error> {
        let mut callbacks = makepkg.callbacks.borrow_mut();
        let ignore_stdout = ignore_stdout || pipe_into.is_some();
        let has_pipe = pipe_into.is_some();

        let mut poll = Poll::new()?;
        let token_in = Token(1 << 0);
        let token_out = Token(1 << 1);
        let token_err = Token(1 << 2);
        let token_err2 = Token(1 << 3);
        let mut events = Events::with_capacity(128);
        let mut buff = vec![0; 1024];
        let mut open = 0;
        let mut insock = None;

        #[derive(Debug, Default)]
        struct CommandData {
            id: usize,
            how_output: callback::CommandOutput,
            outsock: Option<mio::net::UnixStream>,
            errsock: Option<mio::net::UnixStream>,
        }

        let mut setup_out = |command: &mut Command,
                             is_proc2: bool,
                             open: &mut usize|
         -> StdResult<CommandData, io::Error> {
            let mut outsock = None;
            let mut errsock = None;
            let cap_out = (output.is_some() || logfile.is_some()) && !has_pipe;

            let mut id = makepkg.id.borrow_mut();
            *id += 1;
            let id = *id - 1;

            let how_output = if let Some(callbacks) = &mut *callbacks {
                callbacks.command_new(id, kind)
            } else {
                Default::default()
            };

            if matches!(how_output, callback::CommandOutput::Callback) || cap_out {
                let (r, w) = UnixStream::pair()?;
                r.set_nonblocking(true)?;
                let mut r = mio::net::UnixStream::from_std(r);

                if output.is_some() {
                    let (r2, w2) = UnixStream::pair()?;
                    r2.set_nonblocking(true)?;
                    let mut r2 = mio::net::UnixStream::from_std(r2);
                    command.stdout(OwnedFd::from(w2));
                    poll.registry()
                        .register(&mut r2, token_out, Interest::READABLE)?;
                    *open |= token_out.0;
                    outsock = Some(r2);
                } else if !ignore_stdout {
                    let w2 = w.try_clone()?;
                    command.stdout(OwnedFd::from(w2));
                }

                let token = if is_proc2 { token_err2 } else { token_err };

                *open |= token.0;
                poll.registry()
                    .register(&mut r, token, Interest::READABLE)?;
                command.stderr(OwnedFd::from(w));
                errsock = Some(r);
            } else if let callback::CommandOutput::File(ref file) = how_output {
                if !ignore_stdout {
                    command.stdout(file.as_fd().try_clone_to_owned()?);
                }
                command.stderr(file.as_fd().try_clone_to_owned()?);
            } else if let callback::CommandOutput::Null = how_output {
                if !ignore_stdout {
                    command.stdout(Stdio::null());
                }
                command.stderr(Stdio::null());
            };

            let data = CommandData {
                id,
                how_output,
                outsock,
                errsock,
            };

            Ok(data)
        };

        let mut data1 = setup_out(self, false, &mut open)?;
        let mut data2 = Default::default();

        if pipe_into.is_some() {
            self.stdout(Stdio::piped());
        } else if ignore_stdout && output.is_none() {
            self.stdout(Stdio::null());
        }

        if !input.is_empty() {
            let (r, w) = UnixStream::pair()?;
            w.set_nonblocking(true)?;
            let mut w = mio::net::UnixStream::from_std(w);

            self.stdin(OwnedFd::from(r));
            poll.registry()
                .register(&mut w, token_in, Interest::WRITABLE)?;
            open |= token_in.0;
            insock = Some(w);
        } else {
            self.stdin(Stdio::null());
        }

        let mut child = self.spawn()?;
        let mut child2 = None;

        if let Some(command) = pipe_into {
            data2 = setup_out(command, true, &mut open)?;
            command.stdin(child.stdout.take().unwrap());
            child2 = Some(command.spawn()?);
            command.stderr(Stdio::null());
        }

        // make sure the sockets are dropped in out proccess
        self.stdout(Stdio::null());
        self.stderr(Stdio::null());
        let mut ends_with_nl = true;

        while open != 0 {
            poll.poll(&mut events, None)?;
            //println!("open={open}");
            //println!("{events:#?}");

            for event in &events {
                if event.token() == token_in {
                    if let Some(sock) = &mut insock {
                        if event.is_writable() && !input.is_empty() {
                            loop {
                                match sock.write(&mut input) {
                                    Ok(0) => break,
                                    Ok(n) => {
                                        input = &input[n..];
                                        if input.is_empty() {
                                            sock.shutdown(Shutdown::Both)?;
                                            break;
                                        }
                                    }
                                    Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                                    Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                                    Err(e) => return Err(e),
                                }
                            }
                        }
                        if event.is_write_closed() {
                            open = open & !event.token().0;
                        }
                    }
                } else {
                    let data = if event.token() == token_err2 {
                        &mut data2
                    } else {
                        &mut data1
                    };

                    let sock = if event.token() == token_out {
                        &mut data.outsock
                    } else {
                        &mut data.errsock
                    };

                    let how_output = &mut data.how_output;

                    if event.is_readable() {
                        if let Some(sock) = sock {
                            loop {
                                match sock.read(&mut buff) {
                                    Ok(0) => break,
                                    Ok(n) => {
                                        if event.token() == token_out {
                                            if let Some(ref mut out) = output {
                                                out.write_all(&buff[..n])?;
                                            }
                                        }
                                        if let Some(ref mut logfile) = logfile {
                                            logfile.write_all(&buff[..n])?
                                        }
                                        if event.token() != token_out || !ignore_stdout {
                                            ends_with_nl = buff[n - 1] == b'\n';
                                            match how_output {
                                                callback::CommandOutput::Inherit => {
                                                    stdout().write_all(&buff[..n])?
                                                }
                                                callback::CommandOutput::Null => (),
                                                callback::CommandOutput::Callback => {
                                                    if let Some(callbacks) = &mut *callbacks {
                                                        callbacks.command_output(
                                                            data.id,
                                                            kind,
                                                            &buff[..n],
                                                        );
                                                    }
                                                }
                                                callback::CommandOutput::File(ref mut file) => {
                                                    file.write_all(&buff[..n])?
                                                }
                                            }
                                        }
                                    }
                                    Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                                    Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                                    Err(e) => return Err(e),
                                }
                            }
                        }
                    }
                    if event.is_read_closed() {
                        open = open & !event.token().0;

                        if !ends_with_nl && event.token() == token_err {
                            match how_output {
                                callback::CommandOutput::Inherit => stdout().write_all(&[b'\n'])?,
                                callback::CommandOutput::Null => (),
                                callback::CommandOutput::Callback => {
                                    if let Some(callbacks) = &mut *callbacks {
                                        callbacks.command_output(data.id, kind, &[b'\n']);
                                    }
                                }
                                callback::CommandOutput::File(ref mut file) => {
                                    file.write_all(&[b'\n'])?
                                }
                            }
                        }
                    }
                }
            }
        }

        if let Some(callbacks) = &mut *callbacks {
            callbacks.command_exit(data1.id, kind);
        }

        if let Some(mut child2) = child2 {
            let status = child2.wait()?;
            if let Some(callbacks) = &mut *callbacks {
                callbacks.command_exit(data2.id, kind);
            }
            if !status.success() {
                return Ok(status);
            }
        }

        let status = child.wait()?;
        Ok(status)
    }
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

        let workingdir = match function {
            "verify" => dirs.startdir.as_path(),
            _ => dirs.srcdir.as_path(),
        };

        let pkgbase = pkgbuild.pkgbase.as_str();
        let version = pkgbuild.version();
        let pkgdir = &dirs.pkgdir.join(pkgname.unwrap_or(pkgbase));
        let mut output = Vec::new();

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
            .current_dir(&dirs.startdir);

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

        let command_output = if capture_output {
            Some(&mut output)
        } else {
            None
        };

        command
            .process_function(
                self,
                CommandKind::PkgbuildFunction(pkgbuild),
                PKGBUILD_SCRIPT.as_bytes(),
                command_output,
                logfile.as_mut(),
            )
            .cmd_context(&command, Context::RunFunction(function.into()))?;

        let output = String::from_utf8(output)
            .cmd_context(&command, Context::RunFunction(function.into()))?;

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
            .stderr(Stdio::null())
            .stdin(Stdio::null())
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
