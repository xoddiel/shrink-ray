use std::ffi::OsString;
use std::path::{Path, PathBuf};

use tokio::process::Command;

use super::Shrink;

#[derive(Debug)]
pub struct Gm(pub(super) PathBuf);

impl Shrink for Gm {
	fn name(&self) -> &'static str {
		"gm"
	}

	fn extension(&self, _: impl AsRef<Path>) -> &'static str {
		"jpg"
	}

	fn command(&self, input: impl AsRef<Path>, output: impl AsRef<Path>) -> Command {
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
			.arg(output_arg);

		command
	}
}
