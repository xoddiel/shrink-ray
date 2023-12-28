use std::path::{Path, PathBuf};
use std::process::Stdio;

use tokio::process::Command;

use super::Shrink;
use crate::{run_command, Error};

#[derive(Debug)]
pub struct FFMpeg(pub(super) PathBuf);

const TAG: &str = concat!(
	"comment=",
	concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"))
);

impl Shrink for FFMpeg {
	fn get_default_extension(&self, _: impl AsRef<Path>) -> &'static str {
		"webm"
	}

	async fn shrink(&self, input: impl AsRef<Path>, output: impl AsRef<Path>) -> Result<(), Error> {
		let input = input.as_ref();
		let output = output.as_ref();

		let mut command = Command::new(&self.0);
		command
			.args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
			.arg(input)
			.args([
				"-c:v",
				"vp9",
				"-c:a",
				"opus",
				"-strict",
				"-2",
				"-row-mt",
				"1",
				"-map_metadata",
				"-1",
				"-metadata",
				TAG,
				"-f",
				"webm",
			])
			.arg(output)
			.stdin(Stdio::null());

		let status = run_command(command).await?;
		if !status.success() {
			return Err(Error::Conversion("ffmpeg", status));
		}

		Ok(())
	}
}
