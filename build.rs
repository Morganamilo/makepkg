use std::{
    env::var_os,
    ffi::OsStr,
    io::{stdout, Write},
    os::unix::ffi::OsStrExt,
    path::PathBuf,
};

fn set_var<S: AsRef<OsStr>>(var: &str, s: S) {
    let mut stdout = stdout().lock();
    writeln!(stdout, "cargo:rerun-if-env-changed={}", var).unwrap();
    write!(stdout, "cargo:rustc-env={}=", var).unwrap();
    stdout.write_all(s.as_ref().as_bytes()).unwrap();
    writeln!(stdout).unwrap()
}

fn var<P: Into<PathBuf>>(key: &str, or: P) -> PathBuf {
    var_os(key).map(PathBuf::from).unwrap_or(or.into())
}

fn main() {
    println!("cargo:rerun-if-changed=.env");
    let _ = dotenvy::dotenv();

    let prefix = var("PREFIX", "/usr");
    let exec_prefix = var("EXEC_PREFIX", &prefix);
    let libdir = var("LIBDIR", &exec_prefix).join("lib");
    let sysconfdir = var("SYSCONFDIR", "/etc");
    let fakeroot_prefix = var("FAKEROOT_PREFIX", exec_prefix);
    let fakeroot_libsuffix = var("FAKEROOT_LIBSUFFIX", "lib:lib64:lib32");

    let fakeroot_libdirs = fakeroot_libsuffix
        .as_os_str()
        .as_bytes()
        .split(|&c| c == b':')
        .map(|l| {
            fakeroot_prefix
                .join(OsStr::from_bytes(l))
                .join("libfakeroot")
                .into_os_string()
        })
        .collect::<Vec<_>>()
        .join(OsStr::new(":"));

    set_var("PREFIX", prefix);
    set_var("SYSCONFDIR", sysconfdir);
    set_var("LIBDIR", libdir);
    set_var("FAKEROOT_PREFIX", fakeroot_prefix);
    set_var("FAKEROOT_LIBDIRS", fakeroot_libdirs);
}
