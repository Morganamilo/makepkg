//! Raw Pkgbuild is a simple parser for the variables in a pkgbuild file.
//!
//! The aim here is to have a super simple but lossless interface between
//! the bash script that sources the pkgbuild and the rust side that reads
//! the data in. to structs.
//!
//! This parser does not do validation of any kind and allows all sorts of insane
//! value through. This is by design as this is intended to be a very simple
//! layer where we just read the bash values in as is and without losing any data.
//! This then allows for a higher level interface to build on this and implement all
//! the linting and validation without having to think about bash at all.
//!
//! The bash component of this parser reads in all the pkgbuild variables and prints
//! then in a way that we can easily parse. It purposley paraes things like makedepends
//! inside of package functions and pkgver_x86_64 so that the higher level parser can
//! be aware of it and reject the pkgbuild instead of silently ignoring.
//!
//! The data format the bash and rust side use to communicate is internal to this script
//! and puerly an implementation detail. However here is an explanation of the format.
//!
//! The format is line based with each field being separated by a space. Multiple spaces are
//! exquivilent to one space. Each line of output describes one variable or function.
//! Note that strings get surrounded with quotes. Quotes, backslashes and new lines get
//! escaped with `\"`, `\\` and `\n`.
//!
//! The format can be described as:
//!
//! alpha           = "a" | ... | "Z"
//! alpha_num       = alpha | "0" | ... | "9"
//!
//! alpha_under     = "_" | alpha
//! alpha_num_under = "_" | alpha_num
//! function_name   = alpha_under { alpha_num_under }
//!
//! architecture   = alpha_num_under { alpha_num_under }
//! variable_base   = alpha { alpha_num }
//! variable_name   = variable_base [ "_" architecture ]
//!
//! string          = '"' ... '"'
//! key_pair        = string string
//!
//! variable        = "STRING" variable_name string
//!                 | "ARRAY" variable_name { string }
//!                 | "MAP" variable_name { key_pair }
//!
//! global_var      = "GLOBAL"
//! function_var    = "FUNCTION" function_name
//!
//! variable_decl   = "VAR" global_var variable
//!                 | "VAR" function_var variable
//!
//! function_decl   = "FUNCTION" function_name
//!
//! statement        = variable_declration
//!                 | function_declaration
//!
//! statements       = { statement "\n" }
//!
//! Examples:
//!
//! VAR GLOBAL ARRAY arch "x86_64" "aarch64"
//! VAR GLOBAL ARRAY depends "git" "pacman"
//! VAR GLOBAL ARRAY license "GPL3"
//! VAR GLOBAL ARRAY optdepends "bat: colored pkgbuild printing" "devtools: build in chroot and downloading pkgbuilds"
//! VAR GLOBAL STRING pkgrel "1"
//! VAR GLOBAL STRING url "https://github.com/morganamilo/paru"
//! VAR GLOBAL ARRAY depends_x86_64 "gdb"
//! VAR FUNCTION package STRING pkgdesc "does something"
//! VAR FUNCTION package ARRAY depends
//! FUNCTION package

use std::{
    collections::HashMap,
    io::{self, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
};

use crate::{
    error::{CommandError, CommandErrorExt, Context, IOContext},
    FileKind,
};
use crate::{
    error::{Error, IOError},
    pkgbuild::ArchVec,
};

use crate::error::{LintKind, ParseError, ParseErrorKind, Result};

pub(crate) type LintResult<T> = std::result::Result<T, LintKind>;

pub(crate) static PKGBUILD_SCRIPT: &str = include_str!("bash/pkgbuild.sh");

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    String(String),
    Array(Vec<String>),
    Map(HashMap<String, String>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Variable {
    pub name: String,
    pub arch: Option<String>,
    pub value: Value,
}

impl Variable {
    pub fn name_arch(&self) -> String {
        if let Some(arch) = &self.arch {
            format!("{}_{}", self.name, arch)
        } else {
            self.name.clone()
        }
    }

    pub fn assert_no_arch(&self) -> LintResult<()> {
        if self.arch.is_some() {
            return Err(LintKind::CantBeArchitectureSpecific(
                self.name.to_string(),
                self.name_arch(),
            ));
        }

        Ok(())
    }

    pub fn get_arch_array(self) -> LintResult<ArchVec<String>> {
        match self.value {
            Value::Array(a) => Ok(ArchVec::from_vec(self.arch, a)),
            _ => Err(LintKind::WrongValueType(
                self.name_arch(),
                "array".to_string(),
                self.kind().to_string(),
            )),
        }
    }

    pub fn get_array(self) -> LintResult<Vec<String>> {
        self.assert_no_arch()?;
        match self.value {
            Value::Array(a) => Ok(a),
            _ => Err(LintKind::WrongValueType(
                self.name_arch(),
                "array".to_string(),
                self.kind().to_string(),
            )),
        }
    }

    pub fn get_path_array(self) -> LintResult<Vec<PathBuf>> {
        self.get_array()
            .map(|v| v.into_iter().map(PathBuf::from).collect())
    }

    pub fn get_string(self) -> LintResult<String> {
        self.assert_no_arch()?;
        match self.value {
            Value::String(s) => Ok(s),
            _ => Err(LintKind::WrongValueType(
                self.name_arch(),
                "string".to_string(),
                self.kind().to_string(),
            )),
        }
    }

    fn kind(&self) -> &'static str {
        match self.value {
            Value::String(_) => "string",
            Value::Array(_) => "array",
            Value::Map(_) => "map",
        }
    }
}

#[derive(Default, Debug)]
pub struct FunctionVariables {
    pub function_name: String,
    pub variables: Vec<Variable>,
}

#[derive(Default, Debug)]
pub struct RawConfig {
    pub variables: Vec<Variable>,
}

impl RawConfig {
    pub fn from_paths<P: AsRef<Path>>(paths: &[P]) -> Result<Self> {
        let output = bash_output(None, paths, "conf")?;
        let config: RawConfig = RawConfig::parse_processed_output(&output)?;
        Ok(config)
    }

    fn parse_processed_output(s: &str) -> Result<Self> {
        let mut data = RawPkgbuild::default();

        for line in s.lines() {
            parse_line(&mut data, line, FileKind::Config)?;
        }

        let data = RawConfig {
            variables: data.variables,
        };
        Ok(data)
    }
}

#[derive(Default, Debug)]
pub struct RawPkgbuild {
    pub variables: Vec<Variable>,
    pub function_variables: Vec<FunctionVariables>,
    pub functions: Vec<String>,
}

impl RawPkgbuild {
    pub fn from_path<P: AsRef<Path>>(path: P) -> Result<Self> {
        Self::from_path_internal(path)
    }

    fn from_path_internal<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path = path.as_ref();
        let parent = path.parent().ok_or_else(|| {
            IOError::new(
                Context::ReadPkgbuild,
                IOContext::InvalidPath(path.to_path_buf()),
                io::Error::new(io::ErrorKind::InvalidInput, "invalid path"),
            )
        })?;

        let output = bash_output(Some(parent), &[&path], "dump")?;
        let pkgbuild: RawPkgbuild =
            RawPkgbuild::parse_processed_output(&output, FileKind::Pkgbuild)?;
        Ok(pkgbuild)
    }

    fn parse_processed_output(s: &str, file_kind: FileKind) -> Result<Self> {
        let mut data = Self::default();

        for line in s.lines() {
            parse_line(&mut data, line, file_kind)?;
        }

        Ok(data)
    }
}

fn bash_output<P: AsRef<Path>>(dir: Option<&Path>, files: &[P], cmd: &str) -> Result<String> {
    let mut command = Command::new("bash");
    command
        .arg("--noprofile")
        .arg("--norc")
        .arg("-s")
        .arg("-")
        .arg(cmd);
    for file in files {
        command.arg(file.as_ref());
    }
    command.stdin(Stdio::piped());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());

    if let Some(dir) = dir {
        command.current_dir(dir);
    }

    let mut child = command
        .spawn()
        .cmd_context(&command, Context::SourcePkgbuild)?;
    let mut stdin = child.stdin.take().unwrap();

    let thread = thread::spawn(move || stdin.write_all(PKGBUILD_SCRIPT.as_bytes()));

    let output = child
        .wait_with_output()
        .cmd_context(&command, Context::ParsePkgbuild)?;

    thread
        .join()
        .unwrap()
        .map_err(|e| CommandError::exec(e, &command, Context::SourcePkgbuild))?;

    let output = String::from_utf8(output.stdout).cmd_context(&command, Context::SourcePkgbuild)?;

    Ok(output)
}

fn words(line: &str, file_kind: FileKind) -> Result<Vec<String>> {
    let mut words = Vec::new();

    let mut line = line.trim();

    while !line.is_empty() {
        if line.starts_with('"') {
            let mut word = String::new();
            let mut chars = line.chars();
            chars.next();

            loop {
                match chars.next() {
                    Some('\\') => match chars.next() {
                        Some('\\') => word.push('\\'),
                        Some('"') => word.push('"'),
                        Some('n') => word.push('\n'),
                        Some(c) => {
                            return Err(ParseError::new(
                                line,
                                file_kind,
                                ParseErrorKind::UnknownEscapeSequence(c),
                            )
                            .into())
                        }
                        None => todo!(),
                    },
                    Some('"') => break,
                    Some(c) => word.push(c),
                    None => {
                        return Err(ParseError::new(
                            line,
                            file_kind,
                            ParseErrorKind::UnterminatedString(word.to_string()),
                        )
                        .into())
                    }
                }
            }

            if !matches!(chars.next(), None | Some(' ')) {
                return Err(ParseError::new(
                    line,
                    file_kind,
                    ParseErrorKind::UnescapedQuoteInString(word.to_string()),
                )
                .into());
            }

            words.push(word.to_string());
            line = chars.as_str().trim_start()
        } else {
            let (word, rest) = line.split_once(' ').unwrap_or((line, ""));
            words.push(word.to_string());
            line = rest.trim_start();
        }
    }

    Ok(words)
}

fn unexpected_word(line: &str, word: &str, file_kind: FileKind) -> Error {
    ParseError::new(
        line,
        file_kind,
        ParseErrorKind::UnexpectedWord(word.to_string()),
    )
    .into()
}

fn end_of_words<I: Iterator<Item = String>>(
    line: &str,
    file_kind: FileKind,
    words: &mut I,
) -> Result<()> {
    match words.next() {
        Some(w) => Err(unexpected_word(line, &w, file_kind)),
        None => Ok(()),
    }
}

fn next_word<I: Iterator<Item = String>>(
    line: &str,
    file_kind: FileKind,
    words: &mut I,
) -> Result<String> {
    match words.next() {
        Some(word) => Ok(word),
        None => Err(ParseError::new(line, file_kind, ParseErrorKind::UnexpectedEndOfInput).into()),
    }
}

fn parse_line(data: &mut RawPkgbuild, line: &str, file_kind: FileKind) -> Result<()> {
    let mut words = words(line, file_kind)?.into_iter();

    match next_word(line, file_kind, &mut words)?.as_str() {
        "VAR" => {
            let mut conf = false;

            let function = match next_word(line, file_kind, &mut words)?.as_str() {
                "GLOBAL" => None,
                "CONFIG" => {
                    conf = true;
                    None
                }
                "FUNCTION" => Some(next_word(line, file_kind, &mut words)?),
                w => return Err(unexpected_word(line, w, file_kind)),
            };

            let kind = next_word(line, file_kind, &mut words)?;
            let name = next_word(line, file_kind, &mut words)?;

            let (name, arch) = if conf {
                (name, None)
            } else {
                match name.split_once('_') {
                    Some((name, arch)) => (name.to_owned(), Some(arch.to_string())),
                    None => (name, None),
                }
            };

            let value = match kind.as_str() {
                "STRING" => {
                    let value = Value::String(next_word(line, file_kind, &mut words)?);
                    end_of_words(line, file_kind, &mut words)?;
                    value
                }
                "ARRAY" => Value::Array(words.collect()),
                "MAP" => {
                    let mut map = HashMap::new();
                    while let Some(key) = words.next() {
                        let value = next_word(line, file_kind, &mut words)?;
                        map.insert(key, value);
                    }
                    Value::Map(map)
                }
                w => return Err(unexpected_word(line, w, file_kind)),
            };

            let variable = Variable { name, arch, value };

            if let Some(function) = function {
                match data
                    .function_variables
                    .iter_mut()
                    .find(|f| f.function_name == function)
                {
                    Some(f) => f.variables.push(variable),
                    None => data.function_variables.push(FunctionVariables {
                        function_name: function,
                        variables: vec![variable],
                    }),
                }
            } else {
                data.variables.push(variable);
            }
        }
        "FUNCTION" => {
            let function = parse_function(line, file_kind, &mut words)?;
            data.functions.push(function);
        }
        w => return Err(unexpected_word(line, w, file_kind)),
    }

    Ok(())
}

fn parse_function<I: Iterator<Item = String>>(
    line: &str,
    file_kind: FileKind,
    words: &mut I,
) -> Result<String> {
    let word = next_word(line, file_kind, words)?;
    end_of_words(line, file_kind, words)?;
    Ok(word)
}
