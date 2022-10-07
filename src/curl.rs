use crate::*;
use futures::FutureExt;
use std::ffi::OsStr;
use tokio::process;

pub fn get<U, I, S, R, F>(url: U, args: I, f: F) -> Future<R>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
    U: AsRef<OsStr>,
    F: FnOnce(&[u8]) -> Result<R> + std::marker::Send + 'static,
{
    process::Command::new("curl")
        .kill_on_drop(true)
        .args(args)
        .arg("-f")
        .arg(&url)
        .output()
        .map(move |result| match result {
            Ok(output) => match output.status.code() {
                Some(0) => f(&output.stdout),
                Some(n) => Err(Error::Custom(format!(
                    "curl exited with non-0 status: {}. stderr: {:?}",
                    n, output.stderr
                ))),
                None => Err(Error::Custom(format!(
                    "curl terminated by signal. stderr: {:?}",
                    output.stderr
                ))),
            },
            Err(err) => Err(Error::Custom(format!("curl failed to execute: {:?}", err))),
        })
        .boxed()
}
