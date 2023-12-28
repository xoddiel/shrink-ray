use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use tokio::process::Command;

use super::Shrink;
use crate::{run_command, Error};

#[derive(Debug)]
pub struct Gm(pub(super) PathBuf);

impl Shrink for Gm {
	fn get_default_extension(&self, _: impl AsRef<Path>) -> &'static str {
		"jpg"
	}

	async fn shrink(&self, input: impl AsRef<Path>, output: impl AsRef<Path>) -> Result<(), Error> {
		let input = input.as_ref();
		let output = output.as_ref();

		let mut output_arg = OsString::from("jpeg:");
		output_arg.push(output);

		let mut command = Command::new(&self.0);
		command
			.arg("convert")
			.arg(input)
			.arg("-strip")
			.arg("-comment")
			.arg(concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION")))
			.arg(output_arg)
			.stdin(Stdio::null())
			.stdout(Stdio::null());

		let status = run_command(command).await?;
		if !status.success() {
			return Err(Error::Conversion("gm", status));
		}

		Ok(())
	}
}
